# QSONaut Server integration

QSONaut can optionally connect to an independent QSONaut Server using standard
WebSockets. For a hosted installation, configure QSONaut with the normal HTTPS
origin, such as `https://radio.example.net`. The client derives
`wss://radio.example.net/api/v1/ws`; no specialty public port is required.

## Enroll this installation

Create the operator in the server management UI, then exchange that operator's
credentials for a revocable device token:

```bash
curl -sS https://radio.example.net/api/v1/auth/device \
  -H 'Content-Type: application/json' \
  -d '{"callsign":"N0CALL","password":"replace-me","device_name":"shack desktop"}'
```

Copy the returned `token` into a local ignored configuration file or provide it
with `QSONAUT_SERVER_DEVICE_TOKEN`. Do not commit it. A password reset revokes
all tokens for the operator.

```toml
[server]
enabled = true
url = "https://radio.example.net"
device_token = "paste-token-here"
share_presence = true
share_radio_details = false
share_logs = false
```

The switches are independent:

- `share_presence` publishes that this QSONaut installation is online.
- `share_radio_details` additionally publishes radio model, frequency, band,
  mode, and grid metadata. It has no effect unless presence sharing is enabled.
- `share_logs` publishes locally saved contacts with stable idempotency IDs.

All are false by default. QSONaut shows connection state and the
active-event/catalog counts in Operator Profile.

## Automation channels

The same WebSocket carries persisted shared-channel messages. Automations can
observe connection, snapshot, accepted-message, error, and live
`channel_message` events. Recent snapshot traffic is exposed separately as
`channel_history`, so requesting a sync cannot masquerade as new live traffic.
Automations may request a fresh server snapshot with the
`server_read` capability, which is granted to the bundled component by default.

Publishing uses the separate `server_publish` capability and remains disabled
until the operator starts QSONaut with:

```bash
QSONAUT_AUTOMATION_ENABLE_SERVER_PUBLISH=true cargo run -p qsonaut
```

Rule actions are `server_sync` and `server_send_message`; the latter accepts a
templated `channel` and `message`. The server authenticates the device, records
the author, persists the message, and broadcasts it to connected QSONaut
clients over the normal proxy-friendly WebSocket.
