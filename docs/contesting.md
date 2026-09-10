# Contest operation

Choose **Local Contest** or **Field Day** from Operating Activity. Choose the
contest definition in Station Settings. A shared **Contest setup and exchange**
editor appears above the mode workspace, including Voice, CW, FT8, and FT4.
Setup values are saved with the operator profile; received exchange values belong
to the current contact and clear after logging.

Fill the setup fields and check the allowed band/mode feedback. Application-
controlled digital transmissions are blocked by missing/invalid setup or an
unsupported band/mode. Physical radio PTT is outside this GUI guard. Exchange
validation at logging is not yet enforced: verify received fields before logging.
Structured fields describe what was exchanged; they do not add contest message
formats to the FT8/FT4 encoder or transmit entered fields automatically.

**New local session** starts a new occurrence, disarms application TX, resets the
serial to the configured starting value, and separates its duplicate history.
Restarting the app resumes the saved local occurrence. Selecting a new definition
starts another occurrence and clears setup/exchange data. Switching away from a
server event clears its selection. Reconnecting server settings also clears the
selection and disarms TX.

With Dupe check enabled, local contest duplicates match the contest template,
session, station callsign, and catalog rule. `band` permits one contact per band;
`band-mode` distinguishes CW, phone, and digital categories (FT8 and FT4 count in
the same digital category). `none` does not suppress repeats. Legacy contacts
without contest context are not guessed into a session. Imported contacts with
matching context participate normally. Server-wide duplicate decisions remain
pending authoritative scoring support.

Every newly logged QSO stores operator and station separately. In this increment
both use the configured personal call; selecting a club does not authorize or
substitute a club/special call. Event and club IDs are captured when logged.
Editing/publishing a historical record preserves its captured event.

ADIF uses `OPERATOR`, `STATION_CALLSIGN`, `APP_QSONAUT_CONTEST_TEMPLATE_ID`,
`APP_QSONAUT_SESSION_ID`, `APP_QSONAUT_EVENT_ID`, and `APP_QSONAUT_CLUB_ID`.
Older logs load with empty new fields; no operator or entitlement is inferred.

Server-event application-controlled TX is unavailable until event rules and
station authorization can be synchronized. The shared editor explains this
instead of displaying the unrelated local template as the event's rules.

Scores are not calculated by this increment. The catalog's points/formula text
is descriptive and must not be represented as an authoritative contest total.
Server-event template loading, metadata expiry, assigned station identities,
participation management, full exchange gates, scoring and receipts are tracked
in [the phased implementation plan](contesting-implementation-plan.md).
