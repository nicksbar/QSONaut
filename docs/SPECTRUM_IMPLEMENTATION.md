# IC-7300 Spectrum Waterfall Implementation

## Summary

We've wired **real spectrum data** from the IC-7300 to the WebCAT waterfall canvas. The implementation is complete end-to-end:

## Critical startup nuance (IC-7300)

The IC-7300 does **not** continuously emit waterfall/spectrum frames by default after power-up.
You must explicitly send the CI-V enable command after each radio restart (and after reconnects where stream state is uncertain).

### Required stream bootstrap sequence

1. Open serial transport and confirm CI-V communication is alive.
2. Enable scope status:
    - `FE FE <radio_addr> <ctrl_addr> 27 10 01 FD`
3. Enable scope data output:
    - `FE FE <radio_addr> <ctrl_addr> 27 20 01 FD`
4. Request waveform data:
    - `FE FE <radio_addr> <ctrl_addr> 27 00 FD`
5. Wait for incoming waveform frames (`27 00 ...`) before declaring waterfall ready.
4. If no frames arrive within timeout, retry enable once, then surface warning.

### Disable commands

- `FE FE <radio_addr> <ctrl_addr> 27 20 00 FD` (scope data output OFF)
- `FE FE <radio_addr> <ctrl_addr> 27 10 00 FD` (scope status OFF)

### Operational rule of thumb

- Treat spectrum streaming as **session state**, not persistent radio state.
- Always re-issue `27 10 01`, `27 20 01`, then `27 00` on app connect and after radio reboot.
- Do not assume prior successful session means streaming is still active.

### What's Implemented

#### 1. IC-7300 Driver Spectrum Support
**[ui/drivers/icom-ic7300.js](ui/drivers/icom-ic7300.js)**
- `enableSpectrum()` - Sends CI-V commands `0x27 0x10 0x01`, `0x27 0x20 0x01`, then `0x27 0x00`
- `disableSpectrum()` - Sends CI-V commands `0x27 0x20 0x00` and `0x27 0x10 0x00`
- `parseSpectrumLine()` - Decodes IC-7300 spectrum response format (475 data points)
- `getSpectrumData()` - Returns normalized spectrum data (0-100 scale)
- `handleSpectrumLine()` - Ingests incoming spectrum data from serial port

**IC-7300 CI-V Spectrum Protocol:**
```
Frame: FE FE <radio> <ctrl> 0x27 0x00 [waveform payload...] FD
Response: 0-255 per byte mapped to dBm (-130 to -70 dBm)
Data points: 475 values per spectrum line
```

#### 2. HAL Spectrum Polling
**[ui/hal.js](ui/hal.js)**
- Added `spectrumEmitter` for spectrum data events
- Added `spectrumPollInterval` for automatic polling
- `onSpectrum(fn)` - Subscribe to spectrum data updates
- `connect()` - Starts polling loop at driver's `pollingInterval` (200ms for IC-7300)
- `disconnect()` - Clears polling interval and disables scope on radio
- `_ctx()` - Public context for drivers to call handler methods

#### 3. WebSerial Transport Integration
**[ui/app.js](ui/app.js) - Connect button**
- Creates WebSerial transport object with `write()` and `onData()` methods
- Opens serial port at configured baud rate
- Routes incoming data to driver's `handleSpectrumLine()` method
- Automatically starts read loop on connect

#### 4. Waterfall Canvas Rendering
**[ui/app.js](ui/app.js) - initWaterfall()**
- `hal.onSpectrum()` - Subscribes to real spectrum data
- Canvas scrolling - Shifts previous lines up, adds new line at bottom
- Color mapping - Hue and lightness based on dBm value
- Fallback - Mock animation when driver doesn't support spectrum

### Data Flow

```
IC-7300 Radio
    ↓ (CI-V spectrum data via WebSerial)
WebSerial Port
    ↓ (USB data stream)
Transport.onData callback
    ↓
driver.handleSpectrumLine(data, ctx)
    ↓
parseSpectrumLine(data) → parse 475 points
    ↓
spectrumBuffer (queue)
    ↓
Polling Loop (200ms interval)
    ↓
driver.getSpectrumData(ctx)
    ↓
normalize data: 0-255 dBm → 0-100 display
    ↓
hal.spectrumEmitter.emit(specData)
    ↓
ui/app.js: hal.onSpectrum(specData)
    ↓
Canvas rendering:
  - Scroll up (shift pixels)
  - Draw new line at bottom
  - Color by magnitude
```

### Key Design Decisions

1. **Decoupled Polling**
   - Driver just buffers incoming data (`spectrumBuffer`)
   - HAL polls at driver's `pollingInterval` (not every frame)
   - Prevents overwhelming the UI with updates

2. **Normalization**
   - Raw IC-7300: 0-255 bytes per point
   - Normalized: 0-100 for canvas (dBm -130 to -70)
   - Canvas uses HSL: blue (weak) → white (strong)

3. **Transport Agnostic**
   - `transport.onData()` is generic callback
   - Drivers handle their own frame format
   - Supports WebSerial, TCP, USB, any transport

4. **Capability Flags**
   - `schema.capabilities.hasSpectrum = true`
   - `pollingInterval = 200` (IC-7300 recommendation)
   - HAL skips polling if `!hasSpectrum`

5. **Graceful Degradation**
   - Real spectrum if driver supports it
   - Mock animation as fallback
   - No errors if spectrum unavailable

### Testing

#### Without Hardware
Mock animation runs automatically when:
- No driver selected, OR
- Driver connected but doesn't have `getSpectrumData()`, OR
- Driver not connected

#### With IC-7300 Hardware
1. Connect IC-7300 via USB-serial adapter
2. Select "Icom IC-7300" driver
3. Click "Connect", choose serial port
4. Verify bootstrap commands are sent:
    - `TX ... 27 10 01 FD`
    - `TX ... 27 20 01 FD`
    - `TX ... 27 00 FD`
5. Verify stream starts: `RX ... 27 00 ... [waveform bytes] ... FD`
6. Waterfall should show real spectrum data (scrolling)
7. Tune radio to active signal - watch waterfall highlight it
8. Power-cycle radio and reconnect: confirm app re-sends `27 12 00`

#### Console Verification
- Look for log: `"Spectrum scope enabled"` on connect
- Look for TX logs like:
    - `TX FE FE 94 E0 27 10 01 FD`
    - `TX FE FE 94 E0 27 20 01 FD`
    - `TX FE FE 94 E0 27 00 FD`
- Look for RX frames beginning with `... 27 00 ...` after bootstrap
- Spectrum data should update 5x/sec (200ms poll)

### Future Enhancements

1. **Spectrum Center Frequency Tracking**
   - Display frequency range on waterfall axis
   - Align peak with current VFO

2. **Span/Zoom Control**
   - Radio supports variable spans (50kHz, 100kHz, 200kHz, 500kHz)
   - Add UI slider to adjust

3. **Persistence Averaging**
   - IC-7300 `0x27 0x12 0x01` enables averaging
   - Smoother waterfall, less noise

4. **Peak Detection**
   - Mark strongest signals
   - Auto-seek feature

5. **Recording**
   - Save spectrum data to JSONL
   - Replay waterfall history

### Files Modified

- [ui/drivers/icom-ic7300.js](ui/drivers/icom-ic7300.js) - Added spectrum methods (80 lines)
- [ui/hal.js](ui/hal.js) - Added polling + spectrum emitter (20 lines)
- [ui/app.js](ui/app.js) - WebSerial transport + waterfall rendering (100 lines)

### References

- **IC-7300 CI-V Spectrum**: CI-V command 0x27 (scope data)
- **Hamlib Source**: https://github.com/Hamlib/Hamlib/blob/master/rigs/icom/ic7300.c
- **WebSerial API**: https://developer.mozilla.org/en-US/docs/Web/API/Web_Serial_API

---

**Status**: ✅ **Ready to test with real IC-7300**

The waterfall is now a "real boy" - no more mock animation. When you connect an IC-7300:
1. Scope data automatically streams from radio
2. Waterfall displays live spectrum
3. Tune the radio and watch signals appear/disappear
4. Beautiful scrolling display with color-coded strength

Next: Test with actual hardware! 🎙️
