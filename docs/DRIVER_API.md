# WebCAT Driver API Reference

## Overview

WebCAT drivers are ES modules that export a `createDriver()` factory function. Each driver declares its capabilities (modes, controls, commands) and implements a standard interface for radio control.

**Design Philosophy**: Inspired by Hamlib's 25+ year proven architecture, but modernized for JavaScript/WebSerial and web-first deployment.

## Driver Interface

### Required Methods

#### `createDriver(): Driver`
Factory function that returns a driver object.

```javascript
export function createDriver() {
  return {
    id: string,              // Unique ID (e.g., 'icom.ic7300')
    label: string,           // Display name
    schema: CapabilitySchema,
    async connect(opts, ctx),
    async disconnect(ctx),
    async setFrequencyHz(hz, ctx),
    async setMode(mode, ctx),
    async setPTT(on, ctx),
    async applyControl(controlId, value, ctx)
  };
}
```

### CapabilitySchema Object

```typescript
interface CapabilitySchema {
  modes: string[];              // Supported operating modes
  controls: ControlDef[];       // UI control definitions
  initialState: State;          // Default state values
  capabilities?: Capabilities;  // Optional Hamlib-style flags
}
```

### ControlDef Object

```typescript
interface ControlDef {
  id: string;                   // Unique control ID
  kind: 'slider' | 'range' | 'select' | 'toggle' | 'momentary';
  label: string;                // Display label
  group?: string;               // UI grouping (Core, TX, RX, RF, DSP, Fine)
  priority?: number;            // Sort order (higher = first)
  live?: boolean;               // Update on every change (for PTT, etc.)
  
  // For slider/range:
  min?: number;
  max?: number;
  step?: number;
  unit?: string;                // Display unit (%, W, Hz, dB)
  
  // For select:
  options?: string[];           // Valid choices
}
```

### Capabilities Object (Hamlib-inspired)

```typescript
interface Capabilities {
  hasGetFreq?: boolean;         // Can read frequency from radio
  hasSetFreq?: boolean;         // Can set frequency
  hasGetMode?: boolean;         // Can read mode
  hasSetMode?: boolean;         // Can set mode
  hasGetPTT?: boolean;          // Can read PTT state
  hasSetPTT?: boolean;          // Can control PTT
  hasSpectrum?: boolean;        // Radio has spectrum scope
  hasVFOSwap?: boolean;         // Supports VFO A/B swap
  hasSplit?: boolean;           // Supports split operation
  hasRIT?: boolean;             // Has RIT (receiver incremental tuning)
  hasXIT?: boolean;             // Has XIT (transmit incremental tuning)
  targetableVFO?: boolean;      // Can send commands to specific VFO
  pollingInterval?: number;     // Recommended poll rate (ms)
}
```

### Context Object (ctx)

The `ctx` parameter provides HAL interaction:

```typescript
interface Context {
  log(msg: string): void;       // Append to UI console
  setState(patch: Partial<State>): void;  // Update canonical state
  getState(): State;            // Read current state
  emit(): void;                 // Force UI refresh
}
```

### State Object

Canonical radio state (subset):

```typescript
interface State {
  freqHz: number;               // VFO frequency in Hz
  mode: string;                 // Operating mode
  ptt: boolean;                 // Transmit active
  split: boolean;               // Split operation
  power: number;                // RF power (0-100 or watts)
  agc: string;                  // AGC setting
  af: number;                   // Audio level
  rf: number;                   // RF gain
  sql: number;                  // Squelch level
  preamp: boolean | number;     // Preamp on/dB
  att: boolean | number;        // Attenuator on/dB
  nb: boolean | number;         // Noise blanker
  rit: number;                  // RIT offset (Hz)
  xit: number;                  // XIT offset (Hz)
  filter?: string;              // IF filter selection
  [key: string]: any;           // Custom state fields
}
```

## Method Details

### `async connect(opts, ctx)`

Initialize the driver and prepare for communication.

**Parameters:**
- `opts.transport` - Optional transport object for serial I/O (see Transport section)
- `opts.host` - Optional host for network drivers (rigctld)
- `opts.port` - Optional port for network drivers
- `ctx` - Context object

**Responsibilities:**
1. Store transport reference (if provided)
2. Send initialization commands to radio
3. Set initial state via `ctx.setState(schema.initialState)`
4. Log connection status

**Example:**
```javascript
async connect(opts = {}, ctx) {
  this.transport = opts.transport || null;
  ctx.log(`Driver ready: ${this.label}`);
  ctx.setState(this.schema.initialState);
  if (this.transport) {
    await this.transport.write(initCommand);
  }
}
```

### `async disconnect(ctx)`

Clean up resources and close connection.

**Example:**
```javascript
async disconnect(ctx) {
  ctx.log('Disconnecting');
  this.transport = null;
}
```

### `async setFrequencyHz(hz, ctx)`

Set the radio's VFO frequency.

**Parameters:**
- `hz` - Frequency in Hertz (validate and clamp!)
- `ctx` - Context object

**Returns:** `null` or error string

**Responsibilities:**
1. Validate/clamp frequency to radio's supported range
2. Encode frequency command for radio protocol
3. Send command via transport (if available)
4. Update state: `ctx.setState({ freqHz: hz })`

**Example:**
```javascript
async setFrequencyHz(hz, ctx) {
  const safe = Math.max(100000, Math.min(60000000, Number(hz) || 0));
  const cmd = encodeFrequency(safe);
  if (this.transport) {
    await this.transport.write(cmd);
  }
  ctx.setState({ freqHz: safe });
  return null;
}
```

### `async setMode(mode, ctx)`

Set the operating mode.

**Parameters:**
- `mode` - Mode string (must be in `schema.modes`)
- `ctx` - Context object

**Returns:** `null` or error string

**Example:**
```javascript
async setMode(mode, ctx) {
  if (!this.schema.modes.includes(mode)) {
    return `Unsupported mode: ${mode}`;
  }
  const cmd = encodeModeCommand(mode);
  if (this.transport) {
    await this.transport.write(cmd);
  }
  ctx.setState({ mode });
  return null;
}
```

### `async setPTT(on, ctx)`

Control transmit/receive state.

**Parameters:**
- `on` - Boolean (true = transmit, false = receive)
- `ctx` - Context object

**Returns:** `null` or error string

**Example:**
```javascript
async setPTT(on, ctx) {
  const cmd = encodePTTCommand(on);
  if (this.transport) {
    await this.transport.write(cmd);
  }
  ctx.setState({ ptt: !!on });
  return null;
}
```

### `async applyControl(controlId, value, ctx)`

Handle custom control changes (power, RF gain, filters, etc.).

**Parameters:**
- `controlId` - Control ID from `schema.controls`
- `value` - New value (type depends on control kind)
- `ctx` - Context object

**Returns:** State patch object or `null`

**Example:**
```javascript
async applyControl(controlId, value, ctx) {
  let patch = null;
  switch (controlId) {
    case 'power':
      patch = { power: clamp(value, 0, 100) };
      const cmd = encodePowerCommand(patch.power);
      if (this.transport) await this.transport.write(cmd);
      break;
    case 'agc':
      patch = { agc: value };
      const agcCmd = encodeAGCCommand(value);
      if (this.transport) await this.transport.write(agcCmd);
      break;
    // ... handle other controls
    default:
      ctx.log(`Unknown control: ${controlId}`);
      return null;
  }
  if (patch) ctx.setState(patch);
  return patch;
}
```

## Transport Wiring (Optional)

Drivers can optionally use a `transport` object for serial I/O. This is provided in `opts.transport` during `connect()`.

### Transport Interface

```typescript
interface Transport {
  async write(bytes: Uint8Array): Promise<void>;
  async read(): Promise<Uint8Array>;
  onData(callback: (data: Uint8Array) => void): void;
}
```

### WebSerial Transport Example

```javascript
// In HAL controller
const port = await navigator.serial.requestPort();
await port.open({ baudRate: 19200 });

const transport = {
  async write(bytes) {
    const writer = port.writable.getWriter();
    await writer.write(bytes);
    writer.releaseLock();
  },
  onData(callback) {
    const reader = port.readable.getReader();
    (async () => {
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        callback(value);
      }
    })();
  }
};

await driver.connect({ transport }, ctx);
```

## Protocol Helpers

### Icom CI-V (Binary)

```javascript
function buildCIVFrame(payload) {
  const out = new Uint8Array(2 + 2 + payload.length + 1);
  out[0] = 0xFE; out[1] = 0xFE;       // Preamble
  out[2] = radioAddr; out[3] = ctrlAddr;
  out.set(payload, 4);
  out[out.length - 1] = 0xFD;         // End of message
  return out;
}

function encodeFrequency(hz) {
  const data = new Uint8Array(6);
  data[0] = 0x05; // Set frequency command
  // BCD encode 10 digits (LSB first)
  let n = hz;
  for (let i = 0; i < 5; i++) {
    const lo = n % 10; n = Math.floor(n / 10);
    const hi = n % 10; n = Math.floor(n / 10);
    data[1 + i] = ((hi & 0x0F) << 4) | (lo & 0x0F);
  }
  return buildCIVFrame(data);
}
```

### Yaesu CAT (ASCII)

```javascript
function buildYaesuCommand(cmd) {
  return new TextEncoder().encode(cmd + ';');
}

function encodeFrequency(hz) {
  const padded = String(Math.round(hz)).padStart(9, '0');
  return buildYaesuCommand(`FA${padded}`);
}

function encodeMode(mode) {
  const modeMap = { LSB: '01', USB: '02', CW: '03', FM: '04', AM: '05', 'USB-D': '0C' };
  return buildYaesuCommand(`MD0${modeMap[mode] || '02'}`);
}
```

### Kenwood CAT (Binary)

```javascript
function encodeFrequency(hz) {
  const buf = new Uint8Array(5);
  buf[0] = 0x00; // Set freq command
  buf[1] = (hz >> 24) & 0xFF;
  buf[2] = (hz >> 16) & 0xFF;
  buf[3] = (hz >> 8) & 0xFF;
  buf[4] = hz & 0xFF;
  return buf;
}
```

## Best Practices

### 1. Always Validate Inputs
```javascript
async setFrequencyHz(hz, ctx) {
  const safe = Math.max(minFreq, Math.min(maxFreq, Number(hz) || 0));
  // ...
}
```

### 2. Handle Missing Transport Gracefully
```javascript
if (this.transport) {
  await this.transport.write(cmd);
} else {
  ctx.log(`TX: ${hexDump(cmd)}`);
}
```

### 3. Use State Patches for Consistency
```javascript
const patch = { power: value };
ctx.setState(patch);
return patch;
```

### 4. Log Commands for Debugging
```javascript
const hex = Array.from(bytes).map(b => b.toString(16).padStart(2, '0')).join(' ');
ctx.log(`TX: ${hex}`);
```

### 5. Group Controls Logically
- **Core**: mode, ptt, split
- **TX**: power, tune, mic gain
- **RX**: rf, af, sql, agc, filter
- **RF**: preamp, att, ANT selection
- **DSP**: nb, nr, notch
- **Fine**: rit, xit, clarifier

## Testing Without Hardware

Use the mock driver or log commands without transport:

```javascript
export function createDriver() {
  return {
    // ... schema
    async setFrequencyHz(hz, ctx) {
      const cmd = encodeFrequency(hz);
      const hex = Array.from(cmd).map(b => b.toString(16).padStart(2, '0')).join(' ');
      ctx.log(`Would send: ${hex}`);
      ctx.setState({ freqHz: hz });
      return null;
    }
  };
}
```

## Polling for Radio State

Optional `poll()` method for drivers that need to read radio state:

```javascript
export function createDriver() {
  return {
    // ...
    async poll(ctx) {
      if (!this.transport) return;
      
      // Send status query command
      await this.transport.write(statusQueryCmd);
      
      // Parse response (via onData callback) and update state
      // This is radio-specific!
    }
  };
}
```

The HAL can call `driver.poll()` at `schema.capabilities.pollingInterval`.

## Spectrum Data

For radios with spectrum scope (e.g., IC-7300):

```javascript
export function createDriver() {
  return {
    schema: {
      capabilities: { hasSpectrum: true }
    },
    async getSpectrumData(ctx) {
      if (!this.transport) return null;
      
      // Send scope data request (e.g., IC-7300 0x27 command)
      await this.transport.write(scopeRequestCmd);
      
      // Return spectrum line data
      return {
        centerFreqHz: 14074000,
        spanHz: 100000,
        points: [/* array of 475 amplitude values */]
      };
    }
  };
}
```

## Error Handling

Return error strings from methods:

```javascript
async setFrequencyHz(hz, ctx) {
  if (hz < 100000 || hz > 60000000) {
    return `Frequency ${hz} Hz out of range`;
  }
  // ...
  return null; // Success
}
```

The HAL will display errors in the UI.

## Manifest Registration

Add your driver to `ui/drivers/manifest.js`:

```javascript
export const DRIVER_LOADERS = {
  'vendor.model': async () => (await import('./vendor-model.js')).createDriver()
};

export const DRIVER_META = [
  { id: 'vendor.model', label: 'Vendor Model', defaultBaud: 19200 }
];
```

## Resources

- **Hamlib Source**: https://github.com/Hamlib/Hamlib/tree/master/rigs
- **CI-V Spec**: Icom radio manuals, "Remote Control" appendix
- **Yaesu CAT**: FT-991A/FT-857D manuals, CAT command reference
- **Kenwood CAT**: TS-2000 manual, PC control section

---

*WebCAT Driver API v1.0 - January 2026*
