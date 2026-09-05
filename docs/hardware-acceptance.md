# Hardware acceptance validation

This runbook defines the repeatable live-radio acceptance matrix for issue
[#41](https://github.com/nicksbar/QSONaut/issues/41) and
[#46](https://github.com/nicksbar/QSONaut/issues/46). It is intentionally
separate from deterministic CI fixtures: a software test can prove command
handling, but only a connected radio can prove CI-V behavior, USB transport,
waterfall data, and safe recovery.

## Reference setup

The reference station is:

- Radio: Icom IC-7300
- Backend: native Rigwright CI-V
- Port: `/dev/ttyUSB0` (confirm with `--list-radio`; do not assume this path)
- Baud: `115200`
- Radio CI-V address: `0x94` (`148`)
- Controller CI-V address: `0xE0` (`224`)
- Test power: dummy load or antenna system rated for the selected output
- Before PTT: disable any unattended automation and confirm the correct band,
  frequency, mode, and RF-power limit

The operator must record the radio firmware version, USB/CI-V menu settings,
antenna or dummy-load configuration, and whether the test used a real RF load.
Never run the PTT steps into an unknown load.

## Evidence to collect

Each run should retain:

1. The exact commands and their stdout/stderr.
2. QSONaut's diagnostic log from `~/.config/qsonaut/logs/qsonaut.log`.
3. The JSON report from the Rigwright probe, including transport metrics.
4. Radio model, firmware, port, baud, CI-V addresses, date/time, and operator.
5. For every matrix row: `PASS`, `FAIL`, or `BLOCKED`, observed value, expected
   value, and any recovery action.
6. A final readback proving PTT is `OFF` and the radio is left in the agreed
   safe frequency/mode/RF-power state.

Do not include callsign secrets, server tokens, or unrelated personal data in a
shared report. A report that cannot identify the hardware settings is not
reproducible acceptance evidence.

## Preflight and power retest

Build/run from the QSONaut workspace:

```text
cargo run --manifest-path Cargo.toml -p qsonaut -- --list-radio
cargo run --manifest-path Cargo.toml -p qsonaut -- --radio-port /dev/ttyUSB0 --radio-baud 115200 --radio-civ-address 0x94 --controller-civ-address 0xE0 --radio-status
```

The first acceptance action is the power fix retest:

1. Start QSONaut with the native IC-7300 profile and confirm the radio is
   reachable.
2. Use the GUI power control to request `OFF`; record the log line and the
   radio's front-panel state.
3. Use the same control to request `ON`; record the log line and successful
   reconnection/status readback.
4. Repeat once from the opposite initial state. The IC-7300 CI-V power state is
   write-only, so acceptance is based on the physical front panel plus the
   subsequent frequency/mode readback, not a synthetic `get_power` value.

If the power command fails, stop the matrix and capture the complete diagnostic
log before trying other commands.

## Acceptance matrix

Run rows in order. Do not mark a row passed from a GUI label alone; retain the
readback or physical observation.

| ID | Action | Expected evidence |
| --- | --- | --- |
| PWR | Power off/on, as above | Front panel follows both commands; status readback succeeds afterward. |
| RF | Read RF power, write a conservative value such as 10 W, read back, restore the original value | `RfPower` control read/write succeeds and the radio display agrees. |
| FREQ | Read frequency, set a known test frequency, verify, restore | CI-V readback equals the requested Hz and the radio display agrees. |
| MODE | Read mode, set USB then CW or the agreed safe mode, verify, restore | Mode readback matches each write; no unrelated mode/data setting changes. |
| PTT-ON | With the approved load and low RF power, request PTT on | Radio indicates transmit and the PTT readback/event becomes `ON`. |
| PTT-OFF | Request PTT off immediately after the bounded test | Radio returns to receive and readback/event becomes `OFF`. |
| PTT-SAFE | Force the test process/worker to stop while PTT is active, then invoke global disarm/recovery | PTT is guaranteed `OFF`; no queued TX remains; the log contains the recovery result. |
| SCOPE | Enable the native IC-7300 spectrum stream and wait for a first frame; change a supported scope control; disable it | A first frame is received, control change is acknowledged, and disable leaves the radio responsive. |
| LINK-DROP | Disconnect the USB/serial link during a harmless read or after PTT is off | The application reports disconnect/failure without leaving PTT on; no hang. |
| RECONNECT | Reconnect the same device, restart/reconnect the profile, and run status again | The same model/address reconnects and frequency/mode readback succeeds. |
| RECOVER | Issue one deliberately failed harmless command (for example while disconnected), then reconnect and repeat status | Failure is bounded and diagnostic; subsequent valid commands recover without restarting the host. |

For PTT-SAFE, the operator must verify the physical radio state. Software
state alone is insufficient.

## Probe and command references

The model-backed Rigwright probe is the broad read/write evidence pass:

```text
cargo run --manifest-path ../rigwright/Cargo.toml --example ci_v_probe -- /dev/ttyUSB0 115200 --exercise --log artifacts/ic7300-probe.json
```

It reads frequency, mode, PTT, supported controls, meters, repeater/RIT state,
and writes reversible control values before restoring them. It intentionally
skips memory writes and operator-impacting PTT/tuner/scope actions.

Use QSONaut's CLI for bounded individual operations and readback:

```text
cargo run --manifest-path Cargo.toml -p qsonaut -- --radio-port /dev/ttyUSB0 --radio-baud 115200 --radio-civ-address 0x94 --controller-civ-address 0xE0 --set-control rf-power --control-value 10 --verify-after-set
cargo run --manifest-path Cargo.toml -p qsonaut -- --radio-port /dev/ttyUSB0 --radio-baud 115200 --radio-civ-address 0x94 --controller-civ-address 0xE0 --set-frequency-hz 14074000 --set-mode usb --verify-after-set
cargo run --manifest-path Cargo.toml -p qsonaut -- --radio-port /dev/ttyUSB0 --radio-baud 115200 --radio-civ-address 0x94 --controller-civ-address 0xE0 --enable-spectrum-stream --spectrum-timeout-ms 2500
cargo run --manifest-path Cargo.toml -p qsonaut -- --radio-port /dev/ttyUSB0 --radio-baud 115200 --radio-civ-address 0x94 --controller-civ-address 0xE0 --disable-spectrum-stream
```

PTT commands must always be paired with an explicit release, even when the
positive command fails:

```text
cargo run --manifest-path Cargo.toml -p qsonaut -- --radio-port /dev/ttyUSB0 --radio-baud 115200 --radio-civ-address 0x94 --controller-civ-address 0xE0 --ptt on
cargo run --manifest-path Cargo.toml -p qsonaut -- --radio-port /dev/ttyUSB0 --radio-baud 115200 --radio-civ-address 0x94 --controller-civ-address 0xE0 --ptt off
```

## Making this portable to other models

For each new model, keep the same matrix IDs and change only the model-specific
transport/profile facts:

- Add the model's safe port/baud/address setup and documented scope procedure.
- Use the shared `ProbeLog` JSON format; add model-specific records rather than
  replacing common rows.
- Mark unsupported controls as `SKIP` with the driver's reason, not `FAIL`.
- Keep destructive actions opt-in and reversible; never write memories or
  start a tuner sweep by default.
- Store the report beside the release/test artifact with the commit SHA and
  Rigwright/QSONaut versions.
- Add deterministic protocol tests for every discovered failure, but keep the
  physical result as separate acceptance evidence.

## Current run status

On 2026-09-05 this environment could not start the live matrix: no
`/dev/ttyUSB*`, `/dev/ttyACM*`, or `/dev/serial/by-id/*` device was exposed to
the Linux/WSL session. Only virtual `/dev/ttyS*` ports were present. Do not
substitute one of those ports. Once USB passthrough exposes the IC-7300, rerun
preflight and start at `PWR`; this status should then be replaced with the
matrix report and retained JSON probe artifact.
