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

Audio consumers observe the lifecycle in this order:

`Disabled` → `Opening` → `Active` → `Stopping` → `Disabled`

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

Events describe completed ownership transitions. They do not grant the
subscriber permission to mutate the owning state directly.

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