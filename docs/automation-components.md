# QSONaut automation components

QSONaut automation is an event-driven component layer inspired by the playful utility of mIRC scripts. Components subscribe to typed events, match rules, render templates, and propose actions. The host decides which actions are allowed.

The initial foundation lives in `qsonaut-automation`. It does not make network connections or key a radio by itself.

## Event flow

```text
radio / decoder / QSO log / QSONaut Server / connectors
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

Events currently cover decodes, callsign hits, logged QSOs, radio state, contest state, operator profile changes, commands, external messages, QSONaut Server messages, and timers. Fields are intentionally open-ended so protocol- or connector-specific metadata can be added without changing every component.

## Capabilities

Actions are split into explicit capabilities:

- `ui_notification`: show a card, toast, sound request, or other local feedback.
- `external_send`: send a message through a configured Discord, IRC, or future adapter.
- `server_read`: request a fresh server snapshot; event observation still requires an explicit `server_message` subscription.
- `server_publish`: publish a message into a persisted shared server channel.
- `set_compose`: prepare text in a mode's compose box without transmitting it.
- `radio_control`: request tuning or another non-PTT radio operation.
- `transmit`: request an actual RF transmission.

A capability must appear in the component manifest and in the operator's grant set. Missing either check denies the action. `transmit` should remain off by default and eventually require an additional live armed-state check in the GUI executor.

## Rule files

[`automation.example.toml`](../automation.example.toml) demonstrates:

- a callsign-hit notification;
- a qso-logged confirmation notification;
- forwarding that hit to Discord;
- responding locally to an `!rig` external command;
- requesting a server sync and publishing into `#ops` from matched commands;
- reacting to live shared-channel messages;
- Discord and IRC source declarations that reference environment variables rather than storing tokens.

Templates use `${field}` placeholders. `${source}` and `${timestamp_ms}` are always available; other values come from the event fields.

## Connector boundary

`ExternalSource` is the adapter contract shared by Discord, IRC, local sockets, or future services. An adapter reports connection state, polls normalized events, and sends approved messages. Tokens are referenced by environment-variable name in configuration; adapters should resolve them at runtime and never echo them into events or logs.

The source declarations are configuration contracts, not active connector implementations yet. That keeps the first layer testable and avoids coupling the automation model to a specific Discord or IRC library.

## Integration status

The GUI now owns an `AutomationHost` and dispatches normalized events for:

1. configured callsign detections (`callsign_hit`);
2. QSO logging transitions (`qso_logged`);
3. contest profile transitions (`contest_state`);
4. operator profile transitions (`operator_profile`);
5. material radio state transitions (`radio_state`).

Current runtime grants are intentionally conservative:

- `ui_notification` is granted to the sample component by default;
- `server_read` is granted to the sample component by default;
- `server_publish` requires `QSONAUT_AUTOMATION_ENABLE_SERVER_PUBLISH=true`;
- other consequential actions remain denied unless explicitly granted.

QSONaut Server channel receive and publish are live over the configured WebSocket. Messages are authenticated, persisted, broadcast, and normalized into `server_message` events. This is the extensible mIRC-like scripting seam for coordination features without giving a script blanket access to the whole application.

Still pending:

6. live external adapter polling and transport wiring (Discord/IRC runtime connectors).

Safety-gated execution is now wired for approved `radio_command` and `request_transmit` actions:

- radio commands are blocked while TX/PTT is active and disallow direct PTT control;
- TX requests are blocked unless the operator has already armed a TX path and no TX/PTT is active.

External receive-path publication is available today through a GUI-local ingress simulator that emits typed `external_message` events (`source`, `author`, `message`, `channel`) into the same automation pipeline used by future adapters.

Approved actions continue to flow through GUI-owned executors. UI notifications and compose changes are low risk. Radio control and TX must pass the same global armed/disarmed safety gate used by operator controls.
