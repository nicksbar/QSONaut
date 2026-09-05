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
| RF | Read RF power, write a conservative normalized value such as `10`, read back, restore the original value | `RfPower` control read/write succeeds and the radio display agrees. |
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

Use QSONaut's CLI for bounded individual operations and readback. The
`--power` option exercises the same protocol-neutral operation used by the GUI
power control, including the IC-7300 power-on preamble:

```text
cargo run --manifest-path Cargo.toml -p qsonaut -- --radio-port /dev/ttyUSB0 --radio-baud 115200 --radio-civ-address 0x94 --controller-civ-address 0xE0 --power off
cargo run --manifest-path Cargo.toml -p qsonaut -- --radio-port /dev/ttyUSB0 --radio-baud 115200 --radio-civ-address 0x94 --controller-civ-address 0xE0 --power on
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

## Recorded IC-7300 run

Live acceptance was run on 2026-09-05 using `/dev/ttyUSB0`, the stable
CP2102 by-id endpoint, 115200 baud, radio address `0x94`, and controller
address `0xE0`.

| ID | Result | Evidence |
| --- | --- | --- |
| PWR | PASS | `--power off` and `--power on` both completed; a subsequent status read returned `14,074,000 Hz / USB-D`. |
| RF | PASS | `RfPower 10 -> read 10 -> restore 20`; final probe read `RfPower: 20`. |
| FREQ | PASS | `14,074,000 -> 14,075,000 -> 14,074,000 Hz`, with matching readback. |
| MODE | PASS | `USB -> DATA`, with matching readback; final state was USB-D/data. |
| PTT-ON/OFF | PASS | Low-power PTT assertion at normalized RF power `1` succeeded; explicit release succeeded and final probe reported `PTT=false`. RF power was restored to `20`. |
| PTT-SAFE | BLOCKED | The live CLI did not forcibly kill an active worker; deterministic GUI safety tests cover global disarm and transmit-path cleanup. |
| SCOPE | PASS | Native spectrum stream returned a first frame within 5 seconds and disabled cleanly. |
| LINK-DROP | PASS | Status while unplugged returned a bounded `failed to open serial port /dev/ttyUSB0` diagnostic without hanging. |
| RECONNECT | PASS | The stable by-id endpoint and `/dev/ttyUSB0` returned; status readback recovered at `14,074,000 Hz / USB-D`. |
| RECOVER | PASS | Unsupported CI-V request returned `FA`; the next valid status read succeeded. |

Probe artifacts were written to `/tmp/ic7300-probe.json` and
`/tmp/ic7300-final-probe.json` during this run. The reversible exercise probe
reported 74 commands, 73 matched responses, and one timeout during an
unsupported repeater read; the final probe reported 29 commands, 28 matched
responses, and one timeout in the same area. No frame drops were reported.
Copy these files and the QSONaut log to durable release evidence before the
temporary directory is cleared.

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

Retain the JSON probe reports and diagnostic log with the release validation
record.