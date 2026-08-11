# WebCAT Driver Development Guide

## Overview

WebCAT uses a **capability-first driver architecture** inspired by Hamlib. Each driver is a self-contained ES module that declares its capabilities (modes, controls, commands) and implements a standard interface.

## Quick Start: Adding a New Driver

### 1. Copy the Template

```bash
cp ui/drivers/template.js ui/drivers/yaesu-ft991a.js
```

### 2. Fill in the Capabilities

Edit `createDriver()` to define:
- `id` - Unique identifier (e.g., `'yaesu.ft991a'`)
- `label` - Display name
- `schema.modes` - Array of supported modes
- `schema.controls` - UI control definitions
- `schema.initialState` - Default state

### 3. Implement Protocol Commands

Fill in the command builders:
- `connect()` - Initialize radio connection
- `setFrequencyHz()` - Set VFO frequency
- `setMode()` - Set operating mode
- `setPTT()` - Key transmitter
- `applyControl()` - Handle custom controls

### 4. Register in Manifest

Add entry to `ui/drivers/manifest.js`:

```javascript
export const DRIVER_LOADERS = {
  'yaesu.ft991a': async () => (await import('./yaesu-ft991a.js')).createDriver(),
  // ...
};

export const DRIVER_META = [
  { id: 'yaesu.ft991a', label: 'Yaesu FT-991A', defaultBaud: 38400 },
  // ...
];
```

## Driver Interface Contract

### Required Exports

```javascript
export function createDriver() {
  return {
    id: 'vendor.model',
    label: 'Human Readable Name',
    schema: { /* capability definition */ },
    async connect(opts, ctx) { /* setup */ },
    async disconnect(ctx) { /* cleanup */ },
    async setFrequencyHz(hz, ctx) { /* command */ },
    async setMode(mode, ctx) { /* command */ },
    async setPTT(on, ctx) { /* command */ },
    async applyControl(controlId, value, ctx) { /* custom */ }
  };
}
```

### Schema Definition

```javascript
schema: {
  modes: ['LSB', 'USB', 'AM', 'FM', 'CW', 'CW-R', 'RTTY', 'RTTY-R', 'USB-D', 'LSB-D'],
  controls: [
    {
      id: 'power',
      kind: 'slider',
      label: 'RF Power',
      min: 0,
      max: 100,
      step: 1,
      unit: '%',
      group: 'TX',
      priority: 5
    },
    {
      id: 'agc',
      kind: 'select',
      label: 'AGC',
      options: ['FAST', 'MID', 'SLOW', 'OFF'],
      group: 'RX'
    },
    {
      id: 'preamp',
      kind: 'toggle',
      label: 'Preamp',
      group: 'RF'
    },
    {
      id: 'tune',
      kind: 'momentary',
      label: 'Tuner',
      group: 'TX'
    }
  ],
  initialState: {
    freqHz: 14074000,
    mode: 'USB-D',
    ptt: false,
    power: 50,
    agc: 'MID'
  }
}
```

### Control Kinds

| Kind | Use Case | Parameters |
|------|----------|------------|
| `slider` | Continuous value (power, gain) | `min`, `max`, `step`, `unit` |
| `range` | Offset value (RIT, XIT) | `min`, `max`, `step`, `unit` |
| `select` | Enumerated choice (mode, filter) | `options` array |
| `toggle` | On/off feature (preamp, NB) | none |
| `momentary` | Single-shot action (tune) | none |

### Context Object

The `ctx` parameter provides:
- `log(msg)` - Append to console
- `setState(patch)` - Update canonical state
- `getState()` - Read current state
- `emit()` - Force UI update

### Transport Wiring

Drivers can optionally receive a `transport` object for serial I/O:

```javascript
async connect(opts = {}, ctx) {
  this.transport = opts.transport || null;
  if (this.transport) {
    await this.transport.write(initCommand);
  }
  ctx.log('Connected');
}
```

## Protocol Implementation Patterns

### Icom CI-V (Binary)

```javascript
function buildFrame(payload) {
  const out = new Uint8Array(2 + 2 + payload.length + 1);
  out[0] = 0xFE; out[1] = 0xFE;
  out[2] = radioAddr; out[3] = ctrlAddr;
  out.set(payload, 4);
  out[out.length - 1] = 0xFD;
  return out;
}

function encodeFreq(hz) {
  const out = new Uint8Array(6);
  out[0] = 0x05; // command
  // BCD encode frequency...
  return buildFrame(out);
}
```

### Yaesu ASCII CAT

```javascript
function buildCommand(cmd) {
  return new TextEncoder().encode(cmd + ';');
}

function setFrequency(hz) {
  const padded = String(hz).padStart(9, '0');
  return buildCommand(`FA${padded}`);
}
```

### Kenwood Binary CAT

```javascript
function setFrequency(hz) {
  const buf = new Uint8Array(5);
  buf[0] = 0x00; // set freq command
  // Pack frequency into 4 bytes...
  return buf;
}
```

## Mode Definitions (Hamlib Reference)

Common modes from Hamlib:
- **Voice**: `AM`, `FM`, `WFM`, `LSB`, `USB`, `SAM`, `SAL`, `SAH`
- **CW**: `CW`, `CW-R`, `CWN`
- **Digital**: `RTTY`, `RTTY-R`, `PSK`, `PSKR`, `FSK`
- **Data**: `USB-D`, `LSB-D`, `FM-D`, `AM-D`, `PKTUSB`, `PKTLSB`, `PKTFM`
- **Specialty**: `FAX`, `SSTV`, `FT8`, `FT4`, `DSTAR`, `C4FM`, `NXDN`

Always verify against radio manual before adding modes!

## Band Definitions

Standard ham bands (HF + 6m):

```javascript
const BANDS = [
  { name: '160m', min: 1800000, max: 2000000 },
  { name: '80m', min: 3500000, max: 4000000 },
  { name: '60m', min: 5330500, max: 5406500 },
  { name: '40m', min: 7000000, max: 7300000 },
  { name: '30m', min: 10100000, max: 10150000 },
  { name: '20m', min: 14000000, max: 14350000 },
  { name: '17m', min: 18068000, max: 18168000 },
  { name: '15m', min: 21000000, max: 21450000 },
  { name: '12m', min: 24890000, max: 24990000 },
  { name: '10m', min: 28000000, max: 29700000 },
  { name: '6m', min: 50000000, max: 54000000 }
];
```

## Testing Without Hardware

Use the mock driver for testing UI/controls:

```javascript
const driver = await import('./mock.js').then(m => m.createDriver());
// Test control rendering
driver.schema.controls.forEach(c => console.log(c));
// Test command generation (logs only, no serial I/O)
await driver.setFrequencyHz(14074000, { log: console.log, setState: () => {} });
```

## Driver Checklist

Before submitting a new driver:

- [ ] Verified modes against official radio manual
- [ ] Verified commands against official CAT spec or Hamlib source
- [ ] Tested frequency range limits
- [ ] Tested mode switching (especially data modes)
- [ ] Tested PTT on/off
- [ ] Tested custom controls (if any)
- [ ] Added to manifest with correct baud rate
- [ ] Documented any quirks/limitations in driver comments

## Resources

- **Hamlib Source**: https://github.com/Hamlib/Hamlib/tree/master/rigs
- **Icom CI-V**: IC-7300 manual Appendix, "CI-V Reference"
- **Yaesu CAT**: FT-991A manual Chapter 16, "CAT Operation"
- **Kenwood CAT**: TS-2000 manual, "PC Control Command Reference"

## Support

Questions? Check existing drivers:
- `icom-ic7300.js` - Binary CI-V example
- `yaesu-ft991a.js` - ASCII CAT example (when added)
- `mock.js` - Minimal driver template

---
*WebCAT Driver API v1.0 - Inspired by Hamlib's 25+ year proven architecture*
