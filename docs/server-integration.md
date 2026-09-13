# QSONaut Server integration

QSONaut can optionally connect to an independent QSONaut Server using standard
WebSockets. For a hosted installation, configure QSONaut with the normal HTTPS
origin, such as `https://radio.example.net`. The client derives
`wss://radio.example.net/api/v1/ws`; no specialty public port is required.

The desktop defaults to `https://www.qsonaut.com`. Local and self-hosted
installations can replace that endpoint; the client converts `http`/`https`
to the corresponding WebSocket scheme for the live connection.

## Enroll this installation

Use **Link in browser** in the QSONaut Server profile panel when the server
supports the device-authorization endpoints below. The desktop requests a
short-lived device code, opens the verification URI, and polls until the
browser session approves the installation. The device code is never a bearer
credential and the resulting device token is never displayed in the UI.

The server contract is:

```text
POST /api/v1/auth/device/authorize
{
  "client_id": "qsonaut-desktop",
  "device_name": "QSONaut desktop",
  "client_version": "0.4.2"
}

200 OK
{
  "device_code": "...",
  "user_code": "ABCD-EFGH",
  "verification_uri": "https://www.qsonaut.com/link",
  "verification_uri_complete": "https://www.qsonaut.com/link?user_code=ABCD-EFGH",
  "expires_in": 600,
  "interval": 5
}

POST /api/v1/auth/device/token
{
  "client_id": "qsonaut-desktop",
  "device_code": "...",
  "grant_type": "urn:ietf:params:oauth:grant-type:device_code"
}

200 OK
{ "access_token": "...", "token_type": "Bearer" }
```

While approval is pending, the token endpoint returns `400` with
`{"error":"authorization_pending"}`. It may return `slow_down`,
`access_denied`, or `expired_token`; `Retry-After` is honored when present.
The server should bind the issued token to the approved user, device name,
scopes, and revocation lifecycle. Until these endpoints are deployed, the
advanced existing-token field remains available for development.

For environments that still provision tokens manually, the legacy flow is:

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
share_diagnostics = false
share_debug_logs = false
```

The switches are independent:

- `share_presence` publishes that this QSONaut installation is online.
- `share_radio_details` additionally publishes radio model, frequency, band,
  mode, and grid metadata. It has no effect unless presence sharing is enabled.
- `share_logs` publishes locally saved contacts with stable idempotency IDs.
  Each published record includes a current verified managed operating identity.
  If the server catalog cannot resolve one, QSONaut leaves the local record
  intact but does not queue an ambiguous server submission. The server applies
  the callsign, club, and event activity policy rather than accepting a
  client-supplied audience.
- `share_diagnostics` allows the operator to send a manual runtime snapshot.
- `share_debug_logs` adds a bounded, redacted recent application-log tail to
  that manual snapshot. It has no effect unless diagnostics are enabled and
  never causes continuous log upload.

All are false by default. QSONaut shows connection state and the
active-event/catalog counts in Operator Profile.

### QSO log wire shape

Each queued log conforms to the server's `QsoLogInput` contract. The required
identity and contact fields are top-level fields; contest values are carried in
the JSON `exchange` object, with named fields under `fields_sent` and
`fields_received`:

```json
{
  "event_id": "optional-event-uuid-or-null",
  "operating_callsign": "N7UF",
  "callsign_id": "managed-identity-uuid",
  "idempotency_key": "stable-uuid",
  "callsign": "W1AW",
  "band": "20m",
  "mode": "FT8",
  "frequency_hz": 14074000,
  "occurred_at": "2026-09-11T12:00:00Z",
  "rst_sent": "-10",
  "rst_received": "-12",
  "exchange": {
    "sent": "1A WMA",
    "received": "1B EMA",
    "fields_sent": {"class": "1A", "section": "WMA"},
    "fields_received": {"class": "1B", "section": "EMA"},
    "serial_sent": 7,
    "serial_received": 42,
    "grid": "CN84JU",
    "operator_callsign": "K1OP",
    "station_callsign": "N7UF",
    "contest_template_id": "optional-template-id",
    "club_id": "optional-club-id"
  },
  "points": 0,
  "source": "qsonaut"
}
```

Before queueing, QSONaut rejects records without a contact, band, mode, or
operating callsign; rejects malformed explicit identity/event IDs; and checks
the managed identity's active, verified, effective, expiry, and event-binding
state against the recorded QSO time. The server remains authoritative and
revalidates every submission.

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
