# WebCAT Driver Development Quick Reference

## Choose Your Path

```
┌───────────────────────────────────────────────────────────┐
│  Which radio do you want to support?                      │
└─────────────────┬─────────────────────────────────────────┘
                  │
        ┌─────────┴─────────┐
        │   Is it in the    │
        │ Hamlib supported  │◄──── Search: rigctl -l | grep "model"
        │   radios list?    │
        └─────────┬─────────┘
                  │
        ┌─────────┴─────────┐
        │                   │
       YES                 NO
        │                   │
        │         ┌─────────┴─────────┐
        │         │ Do you have the   │
        │         │  CAT protocol     │
        │         │   manual/spec?    │
        │         └─────────┬─────────┘
        │                   │
        │         ┌─────────┴─────────┐
        │         │                   │
        │        YES                 NO
        │         │                   │
        ▼         ▼                   ▼
┌───────────┐ ┌──────────┐  ┌────────────────┐
│  Option 1 │ │ Option 2 │  │   Option 3     │
│           │ │          │  │                │
│  Hamlib   │ │  Native  │  │  Capture from  │
│  Bridge   │ │  Driver  │  │  existing app  │
│           │ │          │  │  + reverse eng │
└─────┬─────┘ └─────┬────┘  └────────┬───────┘
      │             │                 │
      │             │                 │
      ▼             ▼                 ▼
```

---

## Option 1: Hamlib Bridge (5 minutes)

**When to use:** Your radio is in Hamlib's supported list (300+ models)

**Steps:**
```bash
# 1. Install Hamlib
sudo apt install hamlib-utils     # Debian/Ubuntu
brew install hamlib               # macOS

# 2. Find your radio model number
rigctl -l | grep -i "your radio name"
# Example output: 3073  Icom  IC-7300

# 3. Start rigctld
rigctld -m 3073 -r /dev/ttyUSB0 -s 19200

# 4. In WebCAT UI
# - Select "Hamlib Bridge (Any Radio)"
# - Click Connect
# - Use localhost:4532

✅ DONE! Zero code required.
```

**Pros:** Instant, no programming, 300+ radios  
**Cons:** Requires Hamlib installed, extra process, TCP latency  
**Best for:** Quick testing, exotic radios, non-programmers

---

## Option 2: Native Driver (30min - 2 hours)

**When to use:** You want best performance and have CAT protocol docs

### Path A: Use Scaffolding Tool

```bash
# 1. Generate driver
node tools/new-driver.js yaesu ft991a --protocol=yaesu --baud=38400

# 2. Edit ui/drivers/yaesu-ft991a.js
#    - Verify modes array against manual
#    - Implement encodeFrequency()
#    - Implement encodeModeCommand()
#    - Implement encodePTTCommand()

# 3. Test
npm start
# Select your driver, check console logs (hex dumps)

# 4. Connect real radio
# Click "Connect", choose serial port
```

### Path B: Copy Template Manually

```bash
# 1. Copy template
cp ui/drivers/template.js ui/drivers/my-radio.js

# 2. Edit driver file
# - Change id: 'vendor.model'
# - Change label: 'Vendor Model'
# - Update modes array
# - Implement command encoders

# 3. Register in manifest
# Edit ui/drivers/manifest.js:
#   DRIVER_LOADERS: {
#     'vendor.model': async () => (await import('./my-radio.js')).createDriver()
#   }
#   DRIVER_META: [
#     { id: 'vendor.model', label: 'Vendor Model', defaultBaud: 19200 }
#   ]

# 4. Test (same as Path A)
```

**Pros:** Direct control, fast, offline-capable  
**Cons:** Need protocol knowledge, 1-2 hours work  
**Best for:** Production use, optimal performance

---

## Option 3: Reverse Engineering (Advanced)

**When to use:** No CAT manual, but existing software controls the radio

```bash
# 1. Capture serial traffic
# Use Wireshark with USBPcap filter, or:
sudo cat /dev/ttyUSB0 | xxd -c 16

# 2. Analyze protocol
# - Find frequency set command (test with known freqs)
# - Find mode change command (LSB → USB → CW)
# - Find PTT command (key transmitter)
# - Document byte patterns

# 3. Compare with Hamlib
# Check if Hamlib already supports it:
# https://github.com/Hamlib/Hamlib/tree/master/rigs

# 4. Build driver from template (Option 2 Path B)
```

**Pros:** Works when docs don't exist  
**Cons:** Time-consuming, error-prone  
**Best for:** Rare/vintage radios, no other option

---

## Protocol Quick Reference

### Icom CI-V (Binary)

```javascript
// Frame format: FE FE <radio> <ctrl> <cmd> [data...] FD
function buildCIVFrame(payload) {
  const out = new Uint8Array(5 + payload.length);
  out[0] = 0xFE; out[1] = 0xFE;        // Preamble
  out[2] = 0x94;                       // Radio address (IC-7300)
  out[3] = 0xE0;                       // Controller address
  out.set(payload, 4);
  out[out.length - 1] = 0xFD;          // End
  return out;
}

// Set frequency: 0x05 + BCD encoded Hz
// Set mode: 0x06 + mode byte
// PTT: 0x1C 0x00 <on/off>
```

### Yaesu ASCII CAT

```javascript
// Format: <CMD><PARAMS>;
function buildYaesuCommand(cmd) {
  return new TextEncoder().encode(cmd + ';');
}

// Set frequency: FA<9 digits>;
// Example: FA014074000; = 14.074 MHz
// Set mode: MD0<mode>;
// PTT: TX<0|1>;
```

### Kenwood Binary

```javascript
// Command + binary params
// Set frequency: 0x00 + 4-byte frequency (big-endian)
// Set mode: 0x10 + mode byte
// PTT: 0x50 + on/off byte
```

---

## Driver Anatomy (Essential Parts)

```javascript
export function createDriver() {
  return {
    // 1. Identity
    id: 'vendor.model',           // Unique ID
    label: 'Vendor Model',        // Display name
    
    // 2. Capabilities
    schema: {
      modes: ['LSB', 'USB', 'CW', 'FM'],
      controls: [/* UI controls */],
      initialState: {/* defaults */},
      capabilities: {/* Hamlib flags */}
    },
    
    // 3. Lifecycle
    async connect(opts, ctx) { /* setup */ },
    async disconnect(ctx) { /* cleanup */ },
    
    // 4. Core commands (REQUIRED)
    async setFrequencyHz(hz, ctx) { /* encode + send */ },
    async setMode(mode, ctx) { /* encode + send */ },
    async setPTT(on, ctx) { /* encode + send */ },
    
    // 5. Custom controls (OPTIONAL)
    async applyControl(id, value, ctx) { /* handle others */ }
  };
}
```

---

## Testing Without Hardware

All drivers work in "simulation mode":

```javascript
// Commands are logged instead of sent
async setFrequencyHz(hz, ctx) {
  const cmd = encodeFrequency(hz);
  const hex = Array.from(cmd).map(b => b.toString(16).padStart(2,'0')).join(' ');
  ctx.log(`TX: ${hex}`);  // Logs to console, no serial I/O
  ctx.setState({ freqHz: hz });
}
```

**Check output:**
- UI Console panel shows: `TX: fe fe 94 e0 05 00 74 07 14 fd`
- Compare against CAT manual
- Verify before connecting real radio

---

## Checklist Before Submission

- [ ] Modes verified against official manual
- [ ] Commands match CAT spec (not guessed)
- [ ] Frequency range validated (min/max)
- [ ] PTT tested (if supported)
- [ ] Baud rate correct in manifest
- [ ] Capability flags added (hasSpectrum, etc.)
- [ ] Tested without hardware (console logs)
- [ ] Tested with real radio (if available)
- [ ] Quirks documented in comments

---

## Resources

| Resource | URL |
|----------|-----|
| Hamlib Radio List | `rigctl -l` or https://github.com/Hamlib/Hamlib/tree/master/rigs |
| Driver Development Guide | [docs/DRIVER_DEVELOPMENT.md](DRIVER_DEVELOPMENT.md) |
| Driver API Reference | [docs/DRIVER_API.md](DRIVER_API.md) |
| Template Driver | [ui/drivers/template.js](../ui/drivers/template.js) |
| IC-7300 Example | [ui/drivers/icom-ic7300.js](../ui/drivers/icom-ic7300.js) |
| Rigctld Bridge | [ui/drivers/hamlib-rigctld.js](../ui/drivers/hamlib-rigctld.js) |

---

## Summary

| Method | Time | Difficulty | Best For |
|--------|------|------------|----------|
| Hamlib Bridge | 5 min | Easy | Quick testing, any supported radio |
| Scaffolding Tool | 30-60 min | Medium | Production, good docs available |
| Template Copy | 1-2 hrs | Medium | Custom needs, learning |
| Reverse Engineering | 4+ hrs | Hard | No docs, vintage radios |

**Recommended:** Start with Hamlib bridge to test, then build native driver for production use.

---

*WebCAT Driver Development Quick Reference v1.0*
