use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

static LOG_GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();

#[cfg(any(target_os = "windows", target_os = "macos"))]
const APP_DIR_DESKTOP: &str = "QSONaut";
#[cfg(target_os = "linux")]
const APP_DIR_UNIX: &str = "qsonaut";
const LOG_DIR: &str = "logs";
const LOG_FILE: &str = "qsonaut.log";

pub fn app_config_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    if let Some(root) = std::env::var_os("APPDATA") {
        return PathBuf::from(root).join(APP_DIR_DESKTOP);
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(root) = std::env::var_os("XDG_CONFIG_HOME") {
            return PathBuf::from(root).join(APP_DIR_UNIX);
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".config").join(APP_DIR_UNIX);
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join(APP_DIR_DESKTOP);
        }
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

pub fn log_dir() -> PathBuf {
    app_config_dir().join(LOG_DIR)
}

pub fn log_file_path() -> PathBuf {
    log_dir().join(LOG_FILE)
}

/// Read at most the newest `max_bytes` from the application log.
///
/// When reading from the middle of a file, the partial first line is omitted
/// so callers always receive complete log records.
pub fn read_log_tail(max_bytes: usize) -> Result<String> {
    read_file_tail(&log_file_path(), max_bytes)
}

/// Remove all existing application log records while preserving the active
/// logger and its file location.
pub fn clear_log() -> Result<()> {
    let path = log_file_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .with_context(|| format!("truncate {}", path.display()))?;
    Ok(())
}

fn read_file_tail(path: &Path, max_bytes: usize) -> Result<String> {
    if max_bytes == 0 {
        return Ok(String::new());
    }
    let mut file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let len = file
        .metadata()
        .with_context(|| format!("inspect {}", path.display()))?
        .len();
    let start = len.saturating_sub(max_bytes as u64);
    let starts_on_line_boundary = if start == 0 {
        true
    } else {
        file.seek(SeekFrom::Start(start - 1))
            .with_context(|| format!("seek {}", path.display()))?;
        let mut previous = [0_u8; 1];
        file.read_exact(&mut previous)
            .with_context(|| format!("read {}", path.display()))?;
        previous[0] == b'\n'
    };
    file.seek(SeekFrom::Start(start))
        .with_context(|| format!("seek {}", path.display()))?;
    let mut bytes = Vec::with_capacity((len - start) as usize);
    file.read_to_end(&mut bytes)
        .with_context(|| format!("read {}", path.display()))?;
    if !starts_on_line_boundary {
        if let Some(line_end) = bytes.iter().position(|byte| *byte == b'\n') {
            bytes.drain(..=line_end);
        } else {
            bytes.clear();
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub fn hamdb_cache_path() -> PathBuf {
    app_config_dir().join("hamdb.sqlite3")
}

pub fn init(default_filter: &str) -> Result<()> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));

    let log_path = log_file_path();
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    let file_name = log_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(LOG_FILE);
    let file_appender = tracing_appender::rolling::never(log_dir(), file_name);
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    let _ = LOG_GUARD.set(guard);

    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_file(true)
        .with_line_number(true)
        .compact()
        .with_writer(file_writer);

    #[cfg(debug_assertions)]
    let stderr_layer = fmt::layer().with_target(false).compact();

    let registry = tracing_subscriber::registry().with(filter).with(file_layer);

    #[cfg(debug_assertions)]
    registry.with(stderr_layer).try_init()?;

    #[cfg(not(debug_assertions))]
    registry.try_init()?;

    install_panic_hook();
    tracing::info!(log_path = %log_path.display(), "QSONaut logging initialized");
    Ok(())
}

fn install_panic_hook() {
    let log_path = log_file_path();
    std::panic::set_hook(Box::new(move |panic_info| {
        let location = panic_info
            .location()
            .map(|location| format!("{}:{}", location.file(), location.line()))
            .unwrap_or_else(|| "unknown".to_string());
        let message = if let Some(message) = panic_info.payload().downcast_ref::<&str>() {
            (*message).to_string()
        } else if let Some(message) = panic_info.payload().downcast_ref::<String>() {
            message.clone()
        } else {
            "non-string panic payload".to_string()
        };

        tracing::error!(%location, %message, log_path = %log_path.display(), "QSONaut panic");
    }));
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QsoRecord {
    #[serde(default)]
    pub id: u64,
    pub qso_date: String,
    pub time_on: String,
    pub time_off: String,
    pub callsign: String,
    #[serde(default)]
    pub grid: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub hamdb: Option<HamDbCacheEntry>,
    pub band: String,
    pub mode: String,
    pub frequency_hz: u64,
    /// Operating/activity context, such as `General`, `Contest`, or `POTA`.
    #[serde(default)]
    pub operation_mode: String,
    /// POTA role for this QSO, such as `Activator` or `Hunter`.
    #[serde(default)]
    pub pota_role: String,
    /// Primary POTA reference associated with this QSO, for example `US-1091`.
    #[serde(default)]
    pub pota_reference: String,
    /// Human-readable name for the primary POTA reference.
    #[serde(default)]
    pub pota_name: String,
    /// Additional POTA references for multi-park activations.
    #[serde(default)]
    pub pota_references: String,
    #[serde(default)]
    pub report_sent: String,
    #[serde(default)]
    pub report_received: String,
    #[serde(default)]
    pub contest_exchange_sent: String,
    #[serde(default)]
    pub contest_exchange_received: String,
    #[serde(default)]
    pub contest_serial_sent: Option<u32>,
    #[serde(default)]
    pub contest_serial_received: Option<u32>,
    #[serde(default)]
    pub notes: String,
}

impl QsoRecord {
    pub fn new(
        callsign: impl Into<String>,
        mode: impl Into<String>,
        band: impl Into<String>,
        frequency_hz: u64,
        started_at_unix: u64,
        ended_at_unix: u64,
    ) -> Self {
        let (qso_date, time_on) = utc_date_time(started_at_unix);
        let (_, time_off) = utc_date_time(ended_at_unix);
        Self {
            id: ended_at_unix.saturating_mul(1_000),
            qso_date,
            time_on,
            time_off,
            callsign: callsign.into().trim().to_ascii_uppercase(),
            grid: String::new(),
            state: String::new(),
            hamdb: None,
            band: band.into(),
            mode: mode.into().trim().to_ascii_uppercase(),
            frequency_hz,
            operation_mode: "General".to_string(),
            pota_role: String::new(),
            pota_reference: String::new(),
            pota_name: String::new(),
            pota_references: String::new(),
            report_sent: String::new(),
            report_received: String::new(),
            contest_exchange_sent: String::new(),
            contest_exchange_received: String::new(),
            contest_serial_sent: None,
            contest_serial_received: None,
            notes: String::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdifExportFilter {
    #[serde(default)]
    pub date_from: Option<String>,
    #[serde(default)]
    pub date_to: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub band: Option<String>,
}

impl AdifExportFilter {
    fn matches(&self, record: &QsoRecord) -> bool {
        if let Some(date_from) = self.date_from.as_deref() {
            if record.qso_date.as_str() < date_from.trim() {
                return false;
            }
        }
        if let Some(date_to) = self.date_to.as_deref() {
            if record.qso_date.as_str() > date_to.trim() {
                return false;
            }
        }
        if let Some(mode) = self.mode.as_deref() {
            if !record.mode.eq_ignore_ascii_case(mode.trim()) {
                return false;
            }
        }
        if let Some(band) = self.band.as_deref() {
            if !record.band.eq_ignore_ascii_case(band.trim()) {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QsoLog {
    #[serde(default = "qso_log_version")]
    pub version: u8,
    #[serde(default)]
    pub contacts: Vec<QsoRecord>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HamDbCacheEntry {
    pub callsign: String,
    #[serde(default)]
    pub class: String,
    #[serde(default)]
    pub expires: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub grid: String,
    #[serde(default)]
    pub latitude: String,
    #[serde(default)]
    pub longitude: String,
    #[serde(default)]
    pub first_name: String,
    #[serde(default)]
    pub middle_name: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub suffix: String,
    #[serde(default)]
    pub address_line_1: String,
    #[serde(default)]
    pub address_line_2: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub zip: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub fetched_at_unix: u64,
}

pub struct HamDbCache {
    connection: rusqlite::Connection,
}

impl HamDbCache {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        let connection = rusqlite::Connection::open(path)
            .with_context(|| format!("open HamDB cache {}", path.display()))?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS hamdb_callsigns (
                callsign TEXT PRIMARY KEY,
                class TEXT NOT NULL DEFAULT '', expires TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT '',
                grid TEXT NOT NULL DEFAULT '',
                latitude TEXT NOT NULL DEFAULT '', longitude TEXT NOT NULL DEFAULT '',
                first_name TEXT NOT NULL DEFAULT '', middle_name TEXT NOT NULL DEFAULT '',
                name TEXT NOT NULL DEFAULT '', suffix TEXT NOT NULL DEFAULT '',
                address_line_1 TEXT NOT NULL DEFAULT '', address_line_2 TEXT NOT NULL DEFAULT '',
                state TEXT NOT NULL DEFAULT '',
                zip TEXT NOT NULL DEFAULT '',
                country TEXT NOT NULL DEFAULT '',
                fetched_at_unix INTEGER NOT NULL
            );",
        )?;
        for column in [
            "class",
            "expires",
            "status",
            "latitude",
            "longitude",
            "first_name",
            "middle_name",
            "name",
            "suffix",
            "address_line_1",
            "address_line_2",
            "zip",
        ] {
            let _ = connection.execute(
                &format!(
                    "ALTER TABLE hamdb_callsigns ADD COLUMN {column} TEXT NOT NULL DEFAULT ''"
                ),
                [],
            );
        }
        Ok(Self { connection })
    }

    pub fn get_fresh(&self, callsign: &str, now: u64, ttl: u64) -> Result<Option<HamDbCacheEntry>> {
        let mut statement = self.connection.prepare(
            "SELECT callsign, class, expires, status, grid, latitude, longitude, first_name,
                    middle_name, name, suffix, address_line_1, address_line_2, state, zip,
                    country, fetched_at_unix
             FROM hamdb_callsigns WHERE callsign = ?1 AND fetched_at_unix > ?2",
        )?;
        let mut rows = statement.query(rusqlite::params![callsign, now.saturating_sub(ttl)])?;
        rows.next()?
            .map(|row| -> rusqlite::Result<HamDbCacheEntry> {
                Ok(HamDbCacheEntry {
                    callsign: row.get(0)?,
                    class: row.get(1)?,
                    expires: row.get(2)?,
                    status: row.get(3)?,
                    grid: row.get(4)?,
                    latitude: row.get(5)?,
                    longitude: row.get(6)?,
                    first_name: row.get(7)?,
                    middle_name: row.get(8)?,
                    name: row.get(9)?,
                    suffix: row.get(10)?,
                    address_line_1: row.get(11)?,
                    address_line_2: row.get(12)?,
                    state: row.get(13)?,
                    zip: row.get(14)?,
                    country: row.get(15)?,
                    fetched_at_unix: row.get(16)?,
                })
            })
            .transpose()
            .map_err(Into::into)
    }

    pub fn upsert(&self, entry: &HamDbCacheEntry) -> Result<()> {
        self.connection.execute(
            "INSERT INTO hamdb_callsigns
             (callsign,class,expires,status,grid,latitude,longitude,first_name,middle_name,name,
              suffix,address_line_1,address_line_2,state,zip,country,fetched_at_unix)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)
             ON CONFLICT(callsign) DO UPDATE SET class=excluded.class, expires=excluded.expires,
             status=excluded.status, grid=excluded.grid, latitude=excluded.latitude,
             longitude=excluded.longitude, first_name=excluded.first_name, middle_name=excluded.middle_name,
             name=excluded.name, suffix=excluded.suffix, address_line_1=excluded.address_line_1,
             address_line_2=excluded.address_line_2, state=excluded.state, zip=excluded.zip,
             country=excluded.country, fetched_at_unix=excluded.fetched_at_unix",
            rusqlite::params![
                entry.callsign,
                entry.class, entry.expires, entry.status, entry.grid, entry.latitude,
                entry.longitude, entry.first_name, entry.middle_name, entry.name, entry.suffix,
                entry.address_line_1, entry.address_line_2, entry.state, entry.zip, entry.country,
                entry.fetched_at_unix
            ],
        )?;
        Ok(())
    }
}

impl Default for QsoLog {
    fn default() -> Self {
        Self {
            version: qso_log_version(),
            contacts: Vec::new(),
        }
    }
}

fn qso_log_version() -> u8 {
    1
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdifImportSummary {
    pub total_records: usize,
    pub imported: usize,
    pub duplicates: usize,
    pub invalid: usize,
}

impl QsoLog {
    pub fn load(path: &Path) -> Result<Self> {
        let source = match fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    version: qso_log_version(),
                    contacts: Vec::new(),
                });
            }
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        toml::from_str(&source).with_context(|| format!("parse {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        let body = toml::to_string_pretty(self).context("serialize QSO log")?;
        let temporary = path.with_extension("toml.tmp");
        fs::write(&temporary, body).with_context(|| format!("write {}", temporary.display()))?;
        fs::rename(&temporary, path).with_context(|| format!("replace {}", path.display()))?;
        Ok(())
    }

    pub fn export_adif(&self, path: &Path) -> Result<()> {
        fs::write(path, self.to_adif()).with_context(|| format!("write {}", path.display()))
    }

    pub fn export_adif_filtered(&self, path: &Path, filter: &AdifExportFilter) -> Result<()> {
        fs::write(path, self.to_adif_filtered(filter))
            .with_context(|| format!("write {}", path.display()))
    }

    pub fn import_adif(&mut self, path: &Path) -> Result<AdifImportSummary> {
        let source =
            fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        Ok(self.import_adif_from_str(&source))
    }

    pub fn import_adif_from_str(&mut self, source: &str) -> AdifImportSummary {
        let mut summary = AdifImportSummary::default();
        let records = parse_adif_records(source);
        summary.total_records = records.len();

        let mut existing_keys: HashSet<(String, String, String, String, String)> = self
            .contacts
            .iter()
            .map(|contact| {
                adif_duplicate_key(
                    &contact.callsign,
                    &contact.qso_date,
                    &contact.time_on,
                    &contact.band,
                    &contact.mode,
                )
            })
            .collect();

        let mut next_id = self
            .contacts
            .iter()
            .map(|contact| contact.id)
            .max()
            .unwrap_or_default()
            .saturating_add(1);

        for fields in records {
            let Some(call) = fields.get("CALL").map(|value| value.trim()) else {
                summary.invalid = summary.invalid.saturating_add(1);
                continue;
            };
            let Some(qso_date) = fields
                .get("QSO_DATE")
                .and_then(|value| normalize_adif_date(value))
            else {
                summary.invalid = summary.invalid.saturating_add(1);
                continue;
            };
            let Some(time_on) = fields
                .get("TIME_ON")
                .and_then(|value| normalize_adif_time(value))
            else {
                summary.invalid = summary.invalid.saturating_add(1);
                continue;
            };
            let Some(mode) = fields
                .get("MODE")
                .map(|value| value.trim().to_ascii_uppercase())
                .filter(|value| !value.is_empty())
            else {
                summary.invalid = summary.invalid.saturating_add(1);
                continue;
            };

            let frequency_hz = fields
                .get("FREQ")
                .and_then(|value| parse_adif_frequency_hz(value));
            let band = fields
                .get("BAND")
                .map(|value| value.trim().to_ascii_lowercase())
                .filter(|value| !value.is_empty())
                .or_else(|| frequency_hz.and_then(band_from_frequency_hz))
                .unwrap_or_else(|| "unknown".to_string());

            let key = adif_duplicate_key(call, &qso_date, &time_on, &band, &mode);
            if existing_keys.contains(&key) {
                summary.duplicates = summary.duplicates.saturating_add(1);
                continue;
            }

            let time_off = fields
                .get("TIME_OFF")
                .and_then(|value| normalize_adif_time(value))
                .unwrap_or_else(|| time_on.clone());

            let record = QsoRecord {
                id: next_id,
                qso_date,
                time_on,
                time_off,
                callsign: call.to_ascii_uppercase(),
                grid: fields
                    .get("GRIDSQUARE")
                    .map(|value| value.trim().to_ascii_uppercase())
                    .unwrap_or_default(),
                state: fields
                    .get("STATE")
                    .map(|value| value.trim().to_ascii_uppercase())
                    .unwrap_or_default(),
                hamdb: None,
                band,
                mode,
                frequency_hz: frequency_hz.unwrap_or_default(),
                operation_mode: fields
                    .get("QSONAUT_OPERATION_MODE")
                    .cloned()
                    .unwrap_or_else(|| "General".to_string()),
                pota_role: fields.get("QSONAUT_POTA_ROLE").cloned().unwrap_or_default(),
                pota_reference: fields
                    .get("SIG_INFO")
                    .filter(|_| {
                        fields
                            .get("SIG")
                            .is_some_and(|signal| signal.eq_ignore_ascii_case("POTA"))
                    })
                    .cloned()
                    .unwrap_or_default(),
                pota_name: fields.get("QSONAUT_POTA_NAME").cloned().unwrap_or_default(),
                pota_references: fields
                    .get("QSONAUT_POTA_REFERENCES")
                    .cloned()
                    .unwrap_or_default(),
                report_sent: fields
                    .get("RST_SENT")
                    .map(|value| value.trim().to_string())
                    .unwrap_or_default(),
                report_received: fields
                    .get("RST_RCVD")
                    .map(|value| value.trim().to_string())
                    .unwrap_or_default(),
                contest_exchange_sent: fields
                    .get("STX_STRING")
                    .map(|value| value.trim().to_string())
                    .unwrap_or_default(),
                contest_exchange_received: fields
                    .get("SRX_STRING")
                    .map(|value| value.trim().to_string())
                    .unwrap_or_default(),
                contest_serial_sent: fields
                    .get("STX")
                    .and_then(|value| value.trim().parse::<u32>().ok()),
                contest_serial_received: fields
                    .get("SRX")
                    .and_then(|value| value.trim().parse::<u32>().ok()),
                notes: fields
                    .get("COMMENT")
                    .or_else(|| fields.get("NOTES"))
                    .map(|value| value.replace(['\r', '\n'], " ").trim().to_string())
                    .unwrap_or_default(),
            };

            self.contacts.push(record);
            existing_keys.insert(key);
            summary.imported = summary.imported.saturating_add(1);
            next_id = next_id.saturating_add(1);
        }

        summary
    }

    pub fn to_adif(&self) -> String {
        self.to_adif_filtered(&AdifExportFilter::default())
    }

    pub fn to_adif_filtered(&self, filter: &AdifExportFilter) -> String {
        let mut output =
            String::from("Generated by QSONaut\n<ADIF_VER:5>3.1.4 <PROGRAMID:8>QSONaut <EOH>\n");
        for contact in self
            .contacts
            .iter()
            .filter(|contact| filter.matches(contact))
        {
            push_adif(&mut output, "QSO_DATE", &contact.qso_date);
            push_adif(&mut output, "TIME_ON", &contact.time_on);
            push_adif(&mut output, "TIME_OFF", &contact.time_off);
            push_adif(&mut output, "CALL", &contact.callsign);
            push_adif(&mut output, "BAND", &contact.band);
            push_adif(&mut output, "MODE", &contact.mode);
            if !contact.operation_mode.trim().is_empty() {
                push_adif(&mut output, "COMMENT", &contact.operation_mode);
            }
            if !contact.pota_reference.trim().is_empty() {
                push_adif(&mut output, "SIG", "POTA");
                push_adif(&mut output, "SIG_INFO", &contact.pota_reference);
            }
            if contact.frequency_hz > 0 {
                push_adif(
                    &mut output,
                    "FREQ",
                    &format!("{:.6}", contact.frequency_hz as f64 / 1_000_000.0),
                );
            }
            push_adif(&mut output, "GRIDSQUARE", &contact.grid);
            push_adif(&mut output, "STATE", &contact.state);
            if let Some(hamdb) = &contact.hamdb {
                push_adif(
                    &mut output,
                    "COMMENT",
                    &format!(
                        "HamDB: {} {} {} {} {} {}",
                        hamdb.name,
                        hamdb.country,
                        hamdb.state,
                        hamdb.grid,
                        hamdb.latitude,
                        hamdb.longitude
                    ),
                );
            }
            push_adif(&mut output, "RST_SENT", &contact.report_sent);
            push_adif(&mut output, "RST_RCVD", &contact.report_received);
            if let Some(serial) = contact.contest_serial_sent {
                push_adif(&mut output, "STX", &serial.to_string());
            }
            if let Some(serial) = contact.contest_serial_received {
                push_adif(&mut output, "SRX", &serial.to_string());
            }
            push_adif(&mut output, "STX_STRING", &contact.contest_exchange_sent);
            push_adif(
                &mut output,
                "SRX_STRING",
                &contact.contest_exchange_received,
            );
            push_adif(
                &mut output,
                "COMMENT",
                &contact.notes.replace(['\r', '\n'], " "),
            );
            output.push_str("<EOR>\n");
        }
        output
    }
}

fn parse_adif_records(source: &str) -> Vec<BTreeMap<String, String>> {
    let mut records = Vec::new();
    let mut current = BTreeMap::new();
    let bytes = source.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'<' {
            index += 1;
            continue;
        }
        let Some(close_offset) = source[index + 1..].find('>') else {
            break;
        };
        let close = index + 1 + close_offset;
        let token = source[index + 1..close].trim();
        let token_upper = token.to_ascii_uppercase();

        if token_upper == "EOR" {
            if !current.is_empty() {
                records.push(std::mem::take(&mut current));
            }
            index = close + 1;
            continue;
        }
        if token_upper == "EOH" {
            current.clear();
            index = close + 1;
            continue;
        }

        let mut segments = token.split(':');
        let Some(name) = segments.next().map(str::trim) else {
            index = close + 1;
            continue;
        };
        let Some(length_str) = segments.next().map(str::trim) else {
            index = close + 1;
            continue;
        };
        let Ok(length) = length_str.parse::<usize>() else {
            index = close + 1;
            continue;
        };

        let value_start = close + 1;
        let value_end = value_start.saturating_add(length).min(source.len());
        let value = source[value_start..value_end].to_string();
        current.insert(name.to_ascii_uppercase(), value);
        index = value_end;
    }

    if !current.is_empty() {
        records.push(current);
    }

    records
}

fn normalize_adif_date(value: &str) -> Option<String> {
    let digits: String = value.chars().filter(|char| char.is_ascii_digit()).collect();
    (digits.len() == 8).then_some(digits)
}

fn normalize_adif_time(value: &str) -> Option<String> {
    let digits: String = value.chars().filter(|char| char.is_ascii_digit()).collect();
    if digits.len() < 4 {
        return None;
    }
    let mut normalized = digits;
    if normalized.len() >= 6 {
        normalized.truncate(6);
    }
    while normalized.len() < 6 {
        normalized.push('0');
    }
    Some(normalized)
}

fn parse_adif_frequency_hz(value: &str) -> Option<u64> {
    let mhz = value.trim().parse::<f64>().ok()?;
    (mhz > 0.0).then_some((mhz * 1_000_000.0).round() as u64)
}

fn band_from_frequency_hz(frequency_hz: u64) -> Option<String> {
    let band = match frequency_hz {
        1_800_000..=2_000_000 => "160m",
        3_500_000..=4_000_000 => "80m",
        5_000_000..=5_500_000 => "60m",
        7_000_000..=7_300_000 => "40m",
        10_100_000..=10_150_000 => "30m",
        14_000_000..=14_350_000 => "20m",
        18_068_000..=18_168_000 => "17m",
        21_000_000..=21_450_000 => "15m",
        24_890_000..=24_990_000 => "12m",
        28_000_000..=29_700_000 => "10m",
        50_000_000..=54_000_000 => "6m",
        144_000_000..=148_000_000 => "2m",
        420_000_000..=450_000_000 => "70cm",
        _ => return None,
    };
    Some(band.to_string())
}

fn adif_duplicate_key(
    callsign: &str,
    qso_date: &str,
    time_on: &str,
    band: &str,
    mode: &str,
) -> (String, String, String, String, String) {
    (
        callsign.trim().to_ascii_uppercase(),
        qso_date.trim().to_string(),
        time_on.trim().to_string(),
        band.trim().to_ascii_uppercase(),
        mode.trim().to_ascii_uppercase(),
    )
}

fn push_adif(output: &mut String, name: &str, value: &str) {
    if !value.trim().is_empty() {
        output.push_str(&format!("<{name}:{}>{} ", value.len(), value));
    }
}

fn utc_date_time(unix_seconds: u64) -> (String, String) {
    let days = (unix_seconds / 86_400) as i64;
    let seconds = unix_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds / 3_600;
    let minute = (seconds % 3_600) / 60;
    let second = seconds % 60;
    (
        format!("{year:04}{month:02}{day:02}"),
        format!("{hour:02}{minute:02}{second:02}"),
    )
}

fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_tail_is_bounded_and_starts_on_a_complete_line() {
        let thread_name = std::thread::current()
            .name()
            .unwrap_or("test")
            .replace(|character: char| !character.is_ascii_alphanumeric(), "_");
        let path = std::env::temp_dir().join(format!(
            "qsonaut-log-tail-{}-{}.log",
            std::process::id(),
            thread_name
        ));
        fs::write(&path, "first record\nsecond record\nthird record\n").unwrap();
        let tail = read_file_tail(&path, 27).unwrap();
        let _ = fs::remove_file(path);
        assert_eq!(tail, "second record\nthird record\n");
    }

    #[test]
    fn log_tail_handles_empty_limits_boundaries_and_unterminated_lines() {
        let path =
            std::env::temp_dir().join(format!("qsonaut-log-tail-edge-{}.log", std::process::id()));
        fs::write(&path, "one\ntwo\npartial").unwrap();
        assert_eq!(read_file_tail(&path, 0).unwrap(), "");
        assert_eq!(read_file_tail(&path, 100).unwrap(), "one\ntwo\npartial");
        assert_eq!(read_file_tail(&path, 3).unwrap(), "");
        assert_eq!(read_file_tail(&path, 9).unwrap(), "partial");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn creates_utc_record_fields() {
        let record = QsoRecord::new("k1abc", "ft8", "20m", 14_074_000, 0, 65);
        assert_eq!(record.qso_date, "19700101");
        assert_eq!(record.time_on, "000000");
        assert_eq!(record.time_off, "000105");
        assert_eq!(record.callsign, "K1ABC");
    }

    #[test]
    fn adif_contains_standard_contact_fields() {
        let log = QsoLog {
            version: 1,
            contacts: vec![QsoRecord::new(
                "K1ABC",
                "FT8",
                "20m",
                14_074_000,
                1_700_000_000,
                1_700_000_060,
            )],
        };
        let adif = log.to_adif();
        assert!(adif.contains("<CALL:5>K1ABC"));
        assert!(adif.contains("<FREQ:9>14.074000"));
        assert!(adif.contains("<EOR>"));
    }

    #[test]
    fn adif_filters_by_mode_and_band() {
        let mut log = QsoLog::default();
        let mut ft8 = QsoRecord::new(
            "K1ABC",
            "FT8",
            "20m",
            14_074_000,
            1_700_000_000,
            1_700_000_060,
        );
        ft8.qso_date = "20260812".to_string();
        let mut ft4 = QsoRecord::new(
            "W1AW",
            "FT4",
            "40m",
            7_074_000,
            1_700_000_100,
            1_700_000_160,
        );
        ft4.qso_date = "20260812".to_string();
        log.contacts = vec![ft8, ft4];

        let filter = AdifExportFilter {
            date_from: None,
            date_to: None,
            mode: Some("FT8".to_string()),
            band: Some("20m".to_string()),
        };
        let adif = log.to_adif_filtered(&filter);
        assert!(adif.contains("<CALL:5>K1ABC"));
        assert!(!adif.contains("<CALL:4>W1AW"));
    }

    #[test]
    fn adif_filters_honor_trimmed_date_bounds_and_case_insensitive_values() {
        let mut early = QsoRecord::new("K1ABC", "FT8", "20m", 14_074_000, 0, 1);
        early.qso_date = "20260812".to_string();
        let mut late = QsoRecord::new("W1AW", "FT4", "40m", 7_074_000, 0, 1);
        late.qso_date = "20260813".to_string();
        let log = QsoLog {
            version: 1,
            contacts: vec![early, late],
        };
        let filter = AdifExportFilter {
            date_from: Some(" 20260812 ".to_string()),
            date_to: Some("20260812".to_string()),
            mode: Some(" ft8 ".to_string()),
            band: Some(" 20M ".to_string()),
        };
        let adif = log.to_adif_filtered(&filter);
        assert!(adif.contains("<CALL:5>K1ABC"));
        assert!(!adif.contains("W1AW"));
    }

    #[test]
    fn adif_preserves_contest_exchange_fields() {
        let mut record = QsoRecord::new("K1ABC", "FT8", "20m", 14_074_000, 0, 1);
        record.contest_serial_sent = Some(12);
        record.contest_serial_received = Some(34);
        record.contest_exchange_sent = "5NN 012".to_string();
        record.contest_exchange_received = "5NN 034".to_string();
        let log = QsoLog {
            version: 1,
            contacts: vec![record],
        };
        let adif = log.to_adif();
        assert!(adif.contains("<STX:2>12"));
        assert!(adif.contains("<SRX:2>34"));
        assert!(adif.contains("<STX_STRING:7>5NN 012"));
        assert!(adif.contains("<SRX_STRING:7>5NN 034"));
    }

    #[test]
    fn saves_and_loads_toml_log() {
        // Thread names contain `::`, which is not a legal Windows path character.
        let thread_name = std::thread::current()
            .name()
            .unwrap_or("test")
            .replace(|c: char| !c.is_ascii_alphanumeric(), "_");
        let path = std::env::temp_dir().join(format!(
            "qsonaut-qso-log-{}-{}.toml",
            std::process::id(),
            thread_name
        ));
        let log = QsoLog {
            version: 1,
            contacts: vec![QsoRecord::new("K1ABC", "FT8", "20m", 14_074_000, 0, 1)],
        };
        log.save(&path).unwrap();
        let loaded = QsoLog::load(&path).unwrap();
        let _ = fs::remove_file(&path);
        assert_eq!(loaded.contacts, log.contacts);
    }

    #[test]
    fn imports_adif_records_and_skips_duplicates() {
        let adif = "Generated by test\n<ADIF_VER:5>3.1.4 <EOH>\n<CALL:5>K1ABC <QSO_DATE:8>20260812 <TIME_ON:6>010203 <TIME_OFF:6>010259 <MODE:3>FT8 <BAND:3>20m <FREQ:9>14.074000 <RST_SENT:3>-10 <RST_RCVD:3>-08 <COMMENT:5>Hello <EOR>\n<CALL:5>K1ABC <QSO_DATE:8>20260812 <TIME_ON:6>010203 <MODE:3>FT8 <BAND:3>20m <EOR>\n<CALL:5>W1AW <QSO_DATE:8>20260812 <TIME_ON:4>0102 <MODE:3>FT4 <FREQ:9>7.074000 <EOR>\n";
        let mut log = QsoLog::default();

        let summary = log.import_adif_from_str(adif);
        assert_eq!(summary.total_records, 3);
        assert_eq!(summary.imported, 2);
        assert_eq!(summary.duplicates, 1);
        assert_eq!(summary.invalid, 0);
        assert_eq!(log.contacts.len(), 2);
        assert_eq!(log.contacts[0].callsign, "K1ABC");
        assert_eq!(log.contacts[1].callsign, "W1AW");
        assert_eq!(log.contacts[1].time_on, "010200");
        assert_eq!(log.contacts[1].band, "40m");
    }

    #[test]
    fn import_adif_marks_invalid_records() {
        let adif = "<EOH>\n<CALL:5>K1ABC <TIME_ON:6>010203 <MODE:3>FT8 <EOR>\n<CALL:5>K1ABC <QSO_DATE:8>20260812 <TIME_ON:2>01 <MODE:3>FT8 <EOR>\n<CALL:5>K1ABC <QSO_DATE:8>20260812 <TIME_ON:6>010203 <EOR>\n";
        let mut log = QsoLog::default();

        let summary = log.import_adif_from_str(adif);
        assert_eq!(summary.total_records, 3);
        assert_eq!(summary.imported, 0);
        assert_eq!(summary.duplicates, 0);
        assert_eq!(summary.invalid, 3);
        assert!(log.contacts.is_empty());
    }

    #[test]
    fn normalizes_adif_date_time_and_frequency_values() {
        assert_eq!(
            normalize_adif_date("2026-08-29"),
            Some("20260829".to_string())
        );
        assert_eq!(normalize_adif_date("2026082"), None);
        assert_eq!(normalize_adif_time("12:34"), Some("123400".to_string()));
        assert_eq!(
            normalize_adif_time("123456.789"),
            Some("123456".to_string())
        );
        assert_eq!(normalize_adif_time("123"), None);
        assert_eq!(parse_adif_frequency_hz("14.074"), Some(14_074_000));
        assert_eq!(parse_adif_frequency_hz("0"), None);
        assert_eq!(parse_adif_frequency_hz("not-a-frequency"), None);
    }

    #[test]
    fn maps_supported_adif_frequencies_and_rejects_gaps() {
        let cases = [
            (1_800_000, "160m"),
            (3_500_000, "80m"),
            (5_000_000, "60m"),
            (7_000_000, "40m"),
            (10_100_000, "30m"),
            (14_000_000, "20m"),
            (18_068_000, "17m"),
            (21_000_000, "15m"),
            (24_890_000, "12m"),
            (28_000_000, "10m"),
            (50_000_000, "6m"),
            (144_000_000, "2m"),
            (420_000_000, "70cm"),
        ];
        for (frequency, band) in cases {
            assert_eq!(band_from_frequency_hz(frequency).as_deref(), Some(band));
        }
        assert_eq!(band_from_frequency_hz(2_500_000), None);
    }

    #[test]
    fn parses_adif_tokens_case_insensitively_and_ignores_malformed_fields() {
        let records = parse_adif_records(
            "<eoh><call:5>K1ABC <broken><mode:x>FT8 <MODE:3>FT8 <EOR><CALL:4>W1AW",
        );
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].get("CALL").map(String::as_str), Some("K1ABC"));
        assert_eq!(records[0].get("MODE").map(String::as_str), Some("FT8"));
        assert_eq!(records[1].get("CALL").map(String::as_str), Some("W1AW"));
    }

    #[test]
    fn normalizes_duplicate_keys_and_only_emits_nonempty_adif_fields() {
        assert_eq!(
            adif_duplicate_key(" k1abc ", "20260829", "120000", "20m", "ft8"),
            (
                "K1ABC".to_string(),
                "20260829".to_string(),
                "120000".to_string(),
                "20M".to_string(),
                "FT8".to_string()
            )
        );
        let mut output = String::new();
        push_adif(&mut output, "CALL", "K1ABC");
        push_adif(&mut output, "EMPTY", "  ");
        assert_eq!(output, "<CALL:5>K1ABC ");
    }

    #[test]
    fn converts_epoch_seconds_to_utc_across_days_and_leap_years() {
        assert_eq!(
            utc_date_time(86_399),
            ("19700101".to_string(), "235959".to_string())
        );
        assert_eq!(
            utc_date_time(86_400),
            ("19700102".to_string(), "000000".to_string())
        );
        assert_eq!(
            utc_date_time(1_709_164_800),
            ("20240229".to_string(), "000000".to_string())
        );
    }

    #[test]
    fn hamdb_cache_upserts_and_observes_ttl_boundaries() {
        let path =
            std::env::temp_dir().join(format!("qsonaut-hamdb-test-{}.sqlite3", std::process::id()));
        let cache = HamDbCache::open(&path).expect("open cache");
        let entry = HamDbCacheEntry {
            callsign: "K1ABC".to_string(),
            name: "Ada Lovelace".to_string(),
            country: "US".to_string(),
            fetched_at_unix: 100,
            ..HamDbCacheEntry::default()
        };
        cache.upsert(&entry).expect("insert cache entry");
        assert_eq!(
            cache.get_fresh("K1ABC", 110, 20).unwrap(),
            Some(entry.clone())
        );
        assert_eq!(cache.get_fresh("K1ABC", 121, 20).unwrap(), None);

        let mut updated = entry.clone();
        updated.name = "Grace Hopper".to_string();
        updated.fetched_at_unix = 200;
        cache.upsert(&updated).expect("update cache entry");
        assert_eq!(cache.get_fresh("K1ABC", 205, 10).unwrap(), Some(updated));
        assert_eq!(cache.get_fresh("N0CALL", 205, 10).unwrap(), None);
        drop(cache);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn missing_qso_log_loads_empty_and_export_writes_a_file() {
        let base = std::env::temp_dir().join(format!("qsonaut-log-io-{}", std::process::id()));
        let missing = base.with_extension("missing.toml");
        let loaded = QsoLog::load(&missing).expect("missing logs are empty");
        assert!(loaded.contacts.is_empty());

        let export = base.with_extension("adif");
        let log = QsoLog::default();
        log.export_adif(&export).expect("export empty log");
        assert!(fs::read_to_string(&export).unwrap().contains("<EOH>"));
        let _ = fs::remove_file(export);
    }
}
