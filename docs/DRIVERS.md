# WebCAT Driver System

## Making It Easy to Add More Radios

WebCAT uses a **capability-first driver architecture** inspired by Hamlib. Each driver is a self-contained ES module that declares what the radio can do, not how to parse every response byte.

### Three Ways to Support Your Radio

#### 1. Native Driver (Recommended)
Best performance, full control, works offline.

```bash
node tools/new-driver.js yaesu ft991a --protocol=yaesu
# Edit ui/drivers/yaesu-ft991a.js
# Test with: npm start
```

**Pros**: Direct serial control, fast, works anywhere  
**Cons**: Need to implement protocol commands  
**Time**: 1-2 hours for basic functionality

#### 2. Hamlib Rigctld Bridge (Universal Fallback)
Instant support for 300+ radios via Hamlib.

```bash
# Install Hamlib
sudo apt install hamlib-utils  # Linux
brew install hamlib            # macOS

# Find your radio model number
rigctl -l | grep IC-7300

# Start rigctld
rigctld -m 3073 -r /dev/ttyUSB0 -s 19200

# Connect WebCAT to rigctld
# Select "Hamlib Bridge (Any Radio)" in UI
# Use localhost:4532 as connection
```

**Pros**: Zero driver code needed, 300+ radios supported  
**Cons**: Requires Hamlib installed, extra process, TCP latency  
**Time**: 5 minutes setup

#### 3. Copy Template and Customize
Start from a working example.

```bash
cp ui/drivers/template.js ui/drivers/my-radio.js
# Edit schema, implement commands
# Add to ui/drivers/manifest.js
```

**Pros**: Structured starting point, protocol examples included  
**Cons**: Still need protocol knowledge  
**Time**: 30min-2 hours depending on radio complexity

### What You Need to Implement

Every driver needs:
1. **Schema**: Modes, controls, initial state
2. **Commands**: Frequency, mode, PTT
3. **Manifest Entry**: Register driver ID

That's it! No parsing loops, no state machines - the HAL handles that.

### Example: Minimal Driver (50 lines)

```javascript
export function createDriver() {
  return {
    id: 'vendor.model',
    label: 'Vendor Model',
    schema: {
      modes: ['LSB', 'USB', 'CW', 'FM'],
      controls: [
        { id: 'mode', kind: 'select', label: 'Mode', 
          options: ['LSB', 'USB', 'CW', 'FM'], group: 'Core' },
        { id: 'ptt', kind: 'toggle', label: 'PTT', group: 'Core' }
      ],
      initialState: { freqHz: 14074000, mode: 'USB', ptt: false }
    },
    async connect(opts, ctx) {
      this.transport = opts.transport;
      ctx.setState(this.schema.initialState);
    },
    async setFrequencyHz(hz, ctx) {
      const cmd = new Uint8Array([...]); // Your protocol
      if (this.transport) await this.transport.write(cmd);
      ctx.setState({ freqHz: hz });
    },
    async setMode(mode, ctx) {
      const cmd = new Uint8Array([...]); // Your protocol
      if (this.transport) await this.transport.write(cmd);
      ctx.setState({ mode });
    },
    async setPTT(on, ctx) {
      const cmd = new Uint8Array([...]); // Your protocol
      if (this.transport) await this.transport.write(cmd);
      ctx.setState({ ptt: on });
    },
    async applyControl(id, val, ctx) { /* custom controls */ }
  };
}
```

### Available Protocols Examples

| Radio Family | Protocol | Helper | Reference |
|--------------|----------|--------|-----------|
| Icom | CI-V binary | `buildCIVFrame()` | IC-7300 manual Appendix |
| Yaesu | ASCII CAT | `buildYaesuCommand()` | FT-991A Ch.16 |
| Kenwood | Binary CAT | `buildKenwoodFrame()` | TS-2000 PC control |
| Hamlib | TCP ASCII | `rigctld` bridge | Hamlib docs |

### Documentation

- **[Driver Development Guide](docs/DRIVER_DEVELOPMENT.md)** - Step-by-step tutorial
- **[Driver API Reference](docs/DRIVER_API.md)** - Complete interface spec
- **[Template Driver](ui/drivers/template.js)** - Copy-paste starting point
- **[IC-7300 Driver](ui/drivers/icom-ic7300.js)** - CI-V binary example
- **[Rigctld Bridge](ui/drivers/hamlib-rigctld.js)** - Hamlib integration

### Quick Start Checklist

- [ ] Copy `ui/drivers/template.js` to `ui/drivers/<your-radio>.js`
- [ ] Edit `id`, `label`, `modes` array
- [ ] Verify modes against official manual (don't guess!)
- [ ] Implement `encodeFrequency()` for your protocol
- [ ] Implement `encodeModeCommand()` for mode switching
- [ ] Implement `encodePTTCommand()` for TX control
- [ ] Add entry to `ui/drivers/manifest.js`
- [ ] Test: `npm start` → Select your driver → Check console logs

### Testing Without Hardware

All drivers work without hardware! Commands are logged to console as hex dumps:

```
TX: fe fe 94 e0 05 00 74 07 14 fd
```

Compare against your radio's CAT spec to verify correctness before connecting.

### Getting Help

1. Check existing drivers: `ui/drivers/icom-ic7300.js`, `ui/drivers/mock.js`
2. Read protocol examples in `docs/DRIVER_DEVELOPMENT.md`
3. Use scaffolding tool: `node tools/new-driver.js <vendor> <model>`
4. Or just use Hamlib bridge as fallback!

### Driver Quality Standards

Before submitting:
- ✅ Modes verified against official manual
- ✅ Commands tested against official CAT spec or Hamlib source
- ✅ Frequency range validated
- ✅ PTT tested (if supported)
- ✅ Added to manifest with correct baud rate
- ✅ Documented any quirks in driver comments

### Current Drivers

| Driver | Status | Protocol | Notes |
|--------|--------|----------|-------|
| Mock Virtual Radio | ✅ Complete | None | Testing/demo |
| Icom IC-7300 | ✅ Complete | CI-V binary | Full support |
| Icom IC-9700 | 📝 Planned | CI-V binary | VHF/UHF |
| Yaesu FT-991A | 📝 Planned | ASCII CAT | HF+6m/2m/70cm |
| Yaesu FT-857D | 📝 Planned | Binary CAT | HF+6m/2m/70cm |
| Hamlib Rigctld | ✅ Complete | TCP ASCII | 300+ radios |

### Contributing

Pull requests welcome! Each new driver helps the community. See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

---

**Design Goal**: Make adding a new radio as easy as filling in a schema and implementing 3-4 command encoders. No parsing loops, no protocol state machines - the HAL handles that.

**Inspired by**: Hamlib's proven 25+ year backend architecture, modernized for JavaScript and web deployment.
