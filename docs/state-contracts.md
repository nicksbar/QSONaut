# QSONaut state and event contracts

This document defines the ownership and ordering rules that components must
follow during the v0.4 consolidation. It is deliberately narrower than the
architecture overview: these are operational contracts, not an inventory of
implementation details.

## Runtime ownership

Each piece of mutable state has one owner:

| State | Owner | Other components may |
| --- | --- | --- |
| Radio connection, frequency, mode, filter, and PTT | Radio session | Read snapshots and request commands |
| Audio device lifecycle and sample-rate configuration | Audio worker | Consume audio and observe lifecycle events |
| Active mode and activity context | GUI/session coordinator | Render state and request mode actions |
| TX arming and disarm state | Shared TX safety gate | Request arm; never override disarm |
| QSO records | Logging service | Submit immutable records and query copies |
| Profile persistence | Profile/configuration layer | Submit validated updates |

Requests are not state mutations. The owning component validates and applies a
request, then publishes the resulting state or error. A caller must not update
another component's state optimistically.

## Command correlation and outcomes

Worker-facing requests use a `CommandEnvelope` with a stable `CommandId`, a
typed `CommandKind`, and a timeout. The owner may publish `Accepted` while work
is pending, but only the owner may publish the terminal `Completed`,
`Canceled`, `TimedOut`, `Rejected`, or `Failed` result. Every result carries
the original ID; consumers must ignore a result for an unknown or already
terminal ID. A timeout cancels eligibility for the requested action, but does
not claim that hardware completed it.

Command IDs are unique while pending. A duplicate pending ID is rejected and
cannot replace, replay, or mutate the original command.

The GUI radio worker publishes these results on `AppEvent::CommandResult` and
advances its command generation when the worker stops or is replaced. Legacy
internal PTT senders are assigned an envelope at the worker boundary so they
retain the same ownership and shutdown semantics.

Commands are at-least-once at the transport boundary and therefore owners must
make cancellation, disarm, and shutdown idempotent. Replaying a stale command
after a component generation changes is rejected rather than applied.

## TX safety contract

The safety gate is authoritative for every transmit path, including digital
mode workers, CW, SSTV, automation, and queued actions.

1. A transmit request must be rejected unless the path is armed and the radio
   session is available.
2. A disarm request takes precedence over an in-flight or queued arm request.
3. Disarm clears queued and active transmit work before returning.
4. Cancellation and failure must release PTT and leave the gate disarmed.
5. Reconnect does not restore an earlier transmit request or armed state.

UI state is only a presentation of this gate. Disabling a control in the UI is
not a substitute for checking the gate at execution time.

## Audio lifecycle contract

All audio consumers use the typed `AudioFormat` contract. QSONaut's canonical
format is 48,000 Hz, one or two channels, with a positive block size; device
formats are converted at the audio boundary before data reaches monitor,
waterfall, or decoder consumers. A device negotiation failure is a lifecycle
failure and must not silently change the decoder's assumed sample rate.

Audio consumers observe the shared lifecycle vocabulary in this order:

`Starting` → `Ready` → `Stopping` → `Stopped`

An input or output device that remains usable but loses one capability may
enter `Degraded`; a failed or disconnected device must recover through a new
`Starting` generation. The implementation uses the same vocabulary for null,
local, and HostBridge audio so software-only validation exercises the same
contract as hardware operation.

An unavailable device, disconnect, or incompatible sample-rate change follows
the same stopping path. The worker must stop producing buffers, publish a
diagnostic, and release the device before it can be reopened. Consumers must
discard buffers from the previous device generation and may not continue a
decode with stale sample-rate assumptions.

## Mode and activity context

The session coordinator owns the active mode context. Switching mode ends the
previous activity before starting the next one. Mode workers may retain
internal caches only while their context generation matches the active
generation; stale queued actions are rejected after a switch or restart.

## Events and logging

Published events are immutable snapshots. Subscribers may keep or copy an
event, but cannot mutate the event held by another subscriber. A `QsoLogged`
event is emitted only after the record has passed validation and persistence;
failed saves must emit an error instead of a success event.

Logging ownership is split into four explicit stages: the GUI owns the
in-memory working set, the log service owns durable TOML persistence and
duplicate IDs, the ADIF boundary owns import/export conversion, and external
reporting services own any LoTW/server submission. A successful `QsoLogged`
event is emitted only after the durable save succeeds. Backup or restore must
use the same validated record path; a failed backup or restore must not replace
the active working set.

Events describe completed ownership transitions. They do not grant the
subscriber permission to mutate the owning state directly.

Lifecycle events are validated against the shared transition matrix. Repeated
states are valid because delivery is at-least-once; a `Stopped` or `Failed`
generation cannot jump directly to `Ready`, and must restart through
`Starting`. Consumers should retain the last accepted state and ignore stale
events rather than reconstructing state from display strings.

Connector state is scoped to the external transport and must include message
provenance in its detail. Connector failure or disconnection must not disable
manual radio workflows. `AiCapability` describes provider/model availability,
not whether AI is required; unavailable or failed AI remains an optional
degraded capability with a manual fallback.

Each connector transport owns its own `Starting` → `Ready` → `Disconnected` /
`Failed` → `Starting` generation. Incoming messages carry transport, author,
channel, and message provenance; connector events never imply radio or TX
authority. A connector reconnect may resume observation, but it may not replay
an old external action without a fresh automation permission decision.

Automation dispatch publishes an immutable result containing the triggering
event source and approved, denied, and error counts. A denied action is never
replayed automatically; changing grants affects subsequent dispatches only.
Permission changes and revocation therefore have an observable result without
exposing the automation host's private component implementation.

## Shutdown and recovery

Shutdown and failure recovery use the same safety ordering:

1. Stop accepting new work.
2. Disarm TX and cancel queued transmit work.
3. Stop mode and audio workers.
4. Release radio/PTT resources.
5. Persist valid state and publish diagnostics.

Recovery may re-open workers only after the previous generation has stopped.
Missing hardware or an unavailable provider must leave the application usable
for manual workflows and must not replay stale actions.