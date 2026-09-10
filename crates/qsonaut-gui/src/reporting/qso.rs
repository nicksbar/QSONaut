use super::super::*;

pub(crate) fn qso_log_path() -> PathBuf {
    app_config_dir().join(QSO_LOG_FILE)
}

pub(crate) fn qso_adif_path() -> PathBuf {
    app_config_dir().join(QSO_ADIF_FILE)
}

pub(crate) fn qso_timestamp(record: &QsoRecord) -> Option<String> {
    let date = record.qso_date.trim();
    let time = record.time_on.trim();
    if date.len() != 8
        || time.len() < 4
        || !date.bytes().all(|byte| byte.is_ascii_digit())
        || !time.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let seconds = if time.len() >= 6 { &time[4..6] } else { "00" };
    Some(format!(
        "{}-{}-{}T{}:{}:{}Z",
        &date[0..4],
        &date[4..6],
        &date[6..8],
        &time[0..2],
        &time[2..4],
        seconds
    ))
}

impl QsonautGuiApp {
    pub(crate) fn persist_qso_log(&mut self, status_prefix: &str) {
        match self.qso_log.save(&qso_log_path()) {
            Ok(()) => {
                info!(contacts = self.qso_log.contacts.len(), status = %status_prefix, "QSO log saved");
                self.qso_log_status = format!("{status_prefix} {}", QSO_LOG_FILE);
                self.qso_log_dirty = false;
            }
            Err(error) => {
                warn!(error = %error, path = %qso_log_path().display(), "QSO log save failed");
                self.qso_log_status = format!("Log save failed: {error}");
            }
        }
    }

    pub(crate) fn append_qso(&mut self, mut record: QsoRecord, status: &str) {
        record.operator_callsign = self.station_callsign.trim().to_ascii_uppercase();
        record.station_callsign = record.operator_callsign.clone();
        record.server_event_id = self
            .server_active_event
            .as_ref()
            .map(|(id, _)| id.clone())
            .unwrap_or_default();
        record.club_id = self
            .server_active_club
            .as_ref()
            .map(|(id, _)| id.clone())
            .unwrap_or_default();
        if self.contest_enabled {
            record.operation_mode = if self.server_active_event.is_some() {
                "Server Contest".to_string()
            } else {
                "Local Contest".to_string()
            };
            record.contest_session_id = self.contest_session_id.clone();
            record.contest_template_id = crate::contest_catalog::find(&self.contest_type)
                .filter(|_| self.server_active_event.is_none())
                .map(|definition| definition.id.clone())
                .unwrap_or_default();
            for (key, value) in self.contest_fields_sent() {
                record.contest_fields_sent.entry(key).or_insert(value);
            }
            for (key, value) in self.contest_fields_received(&record.contest_exchange_received) {
                record.contest_fields_received.entry(key).or_insert(value);
            }
            record
                .contest_serial_sent
                .get_or_insert(self.contest_serial_current.max(1));
            self.contest_serial_current = self
                .contest_serial_current
                .max(record.contest_serial_sent.unwrap_or_default());
        }
        if record.contest_fields_sent.is_empty() {
            record.contest_fields_sent = parse_contest_fields(&record.contest_exchange_sent);
        }
        if record.contest_fields_received.is_empty() {
            record.contest_fields_received =
                parse_contest_fields(&record.contest_exchange_received);
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        let cache = HamDbCache::open(&hamdb_cache_path()).ok();
        if let Some(cache) = cache.as_ref() {
            enrich_qso_from_hamdb(&mut record, cache, now);
        }
        if cache
            .as_ref()
            .and_then(|cache| {
                cache
                    .get_fresh(&record.callsign, now, HAMDB_CACHE_TTL_SECONDS)
                    .ok()
            })
            .flatten()
            .is_none()
        {
            self.hamdb_lookup_rx = Some(spawn_hamdb_lookup(record.callsign.clone(), now));
        }
        if self
            .qso_log
            .contacts
            .iter()
            .any(|contact| contact.id == record.id)
        {
            record.id = self
                .qso_log
                .contacts
                .iter()
                .map(|contact| contact.id)
                .max()
                .unwrap_or_default()
                .saturating_add(1);
        }
        self.qso_log.contacts.push(record);
        if self.contest_enabled {
            self.advance_contest_serial();
            self.profile_dirty = true;
            self.persist_profile("Contest serial saved");
            for field in &mut self.contest_exchange_fields {
                field.received.clear();
            }
        }
        let published = self.qso_log.contacts.last().cloned();
        if let Some(last) = &published {
            self.app_events.publish(AppEvent::QsoLogged {
                mode: last.mode.clone(),
                call: last.callsign.clone(),
                band: last.band.clone(),
                frequency_hz: last.frequency_hz,
                grid: last.grid.clone(),
                state: last.state.clone(),
                country: last
                    .hamdb
                    .as_ref()
                    .map(|entry| entry.country.clone())
                    .unwrap_or_default(),
                time_on: last.time_on.clone(),
                report_received: last.report_received.clone(),
                operation_mode: last.operation_mode.clone(),
                contest_exchange_received: last.contest_exchange_received.clone(),
            });
        }
        self.qso_selected = self.qso_log.contacts.last().map(|contact| contact.id);
        self.qso_log_dirty = true;
        self.persist_qso_log(status);
        if let Some(record) = &published {
            if let Some(bridge) = &self.third_party_bridge {
                bridge.publish(record.clone());
            }
            self.publish_qso_to_server(record);
        }
    }
}

fn parse_contest_fields(exchange: &str) -> std::collections::BTreeMap<String, String> {
    exchange
        .split_whitespace()
        .filter_map(|item| item.split_once('='))
        .filter(|(key, value)| !key.trim().is_empty() && !value.trim().is_empty())
        .map(|(key, value)| (key.trim().to_ascii_uppercase(), value.trim().to_string()))
        .collect()
}
