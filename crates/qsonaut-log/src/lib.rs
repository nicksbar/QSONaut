use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
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
    pub band: String,
    pub mode: String,
    pub frequency_hz: u64,
    #[serde(default)]
    pub report_sent: String,
    #[serde(default)]
    pub report_received: String,
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
            band: band.into(),
            mode: mode.into().trim().to_ascii_uppercase(),
            frequency_hz,
            report_sent: String::new(),
            report_received: String::new(),
            notes: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QsoLog {
    #[serde(default = "qso_log_version")]
    pub version: u8,
    #[serde(default)]
    pub contacts: Vec<QsoRecord>,
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
                band,
                mode,
                frequency_hz: frequency_hz.unwrap_or_default(),
                report_sent: fields
                    .get("RST_SENT")
                    .map(|value| value.trim().to_string())
                    .unwrap_or_default(),
                report_received: fields
                    .get("RST_RCVD")
                    .map(|value| value.trim().to_string())
                    .unwrap_or_default(),
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
        let mut output =
            String::from("Generated by QSONaut\n<ADIF_VER:5>3.1.4 <PROGRAMID:8>QSONaut <EOH>\n");
        for contact in &self.contacts {
            push_adif(&mut output, "QSO_DATE", &contact.qso_date);
            push_adif(&mut output, "TIME_ON", &contact.time_on);
            push_adif(&mut output, "TIME_OFF", &contact.time_off);
            push_adif(&mut output, "CALL", &contact.callsign);
            push_adif(&mut output, "BAND", &contact.band);
            push_adif(&mut output, "MODE", &contact.mode);
            if contact.frequency_hz > 0 {
                push_adif(
                    &mut output,
                    "FREQ",
                    &format!("{:.6}", contact.frequency_hz as f64 / 1_000_000.0),
                );
            }
            push_adif(&mut output, "GRIDSQUARE", &contact.grid);
            push_adif(&mut output, "RST_SENT", &contact.report_sent);
            push_adif(&mut output, "RST_RCVD", &contact.report_received);
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
    fn saves_and_loads_toml_log() {
        let path = std::env::temp_dir().join(format!(
            "qsonaut-qso-log-{}-{}.toml",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
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
}
