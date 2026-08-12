# QSONaut automation components

QSONaut automation is an event-driven component layer inspired by the playful utility of mIRC scripts. Components subscribe to typed events, match rules, render templates, and propose actions. The host decides which actions are allowed.

The initial foundation lives in `qsonaut-automation`. It does not make network connections or key a radio by itself.

## Event flow

```text
radio / decoder / QSO log / Discord / IRC
                    |
                    v
             AutomationEvent
                    |
                    v
       component subscriptions + rules
                    |
                    v
             proposed Actions
                    |
                    v
      manifest request AND operator grant
                    |
          +---------+---------+
          |                   |
       approved             denied + logged
```

Events currently cover decodes, callsign hits, logged QSOs, radio state, commands, external messages, and timers. Fields are intentionally open-ended so protocol- or connector-specific metadata can be added without changing every component.

## Capabilities

Actions are split into explicit capabilities:

- `ui_notification`: show a card, toast, sound request, or other local feedback.
- `external_send`: send a message through a configured Discord, IRC, or future adapter.
- `set_compose`: prepare text in a mode's compose box without transmitting it.
- `radio_control`: request tuning or another non-PTT radio operation.
- `transmit`: request an actual RF transmission.

A capability must appear in the component manifest and in the operator's grant set. Missing either check denies the action. `transmit` should remain off by default and eventually require an additional live armed-state check in the GUI executor.

## Rule files

[`automation.example.toml`](../automation.example.toml) demonstrates:

- a callsign-hit notification;
- forwarding that hit to Discord;
- responding locally to an `!rig` external command;
- Discord and IRC source declarations that reference environment variables rather than storing tokens.

Templates use `${field}` placeholders. `${source}` and `${timestamp_ms}` are always available; other values come from the event fields.

## Connector boundary

`ExternalSource` is the adapter contract shared by Discord, IRC, local sockets, or future services. An adapter reports connection state, polls normalized events, and sends approved messages. Tokens are referenced by environment-variable name in configuration; adapters should resolve them at runtime and never echo them into events or logs.

The source declarations are configuration contracts, not active connector implementations yet. That keeps the first layer testable and avoids coupling the automation model to a specific Discord or IRC library.

## Next integration seam

The GUI should own an `AutomationHost` and publish normalized events after:

1. a decode is accepted;
2. the configured callsign is detected;
3. a QSO is logged;
4. radio state materially changes;
5. an external adapter receives a message.

Approved actions then go through GUI-owned executors. UI notifications and compose changes are low risk. External sends require a configured adapter. Radio control and TX must additionally pass the same global armed/disarmed safety gate used by the operator controls.

