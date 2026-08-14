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

All are false by default. The server connection cannot control the radio,
audio, modem, PTT, transmit scheduling, or automation. QSONaut shows connection
state and the active-event/catalog counts in Operator Profile.
