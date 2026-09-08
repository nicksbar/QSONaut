use super::super::*;
use qsonaut_core::LogOutcome;

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
    pub(crate) fn persist_qso_log(&mut self, status_prefix: &str) -> LogOutcome {
        if !matches!(
            self.component_states.get(&Component::Logging),
            Some(ComponentState::Starting | ComponentState::Ready)
        ) {
            self.publish_component_state(
                Component::Logging,
                ComponentState::Starting,
                format!("saving QSO log: {status_prefix}"),
            );
        }
        match self.qso_log.save(&qso_log_path()) {
            Ok(()) => {
                info!(contacts = self.qso_log.contacts.len(), status = %status_prefix, "QSO log saved");
                self.qso_log_status = format!("{status_prefix} {}", QSO_LOG_FILE);
                self.qso_log_dirty = false;
                self.publish_component_state(
                    Component::Logging,
                    ComponentState::Ready,
                    format!("{} contacts persisted", self.qso_log.contacts.len()),
                );
                LogOutcome::Persisted
            }
            Err(error) => {
                warn!(error = %error, path = %qso_log_path().display(), "QSO log save failed");
                self.qso_log_status = format!("Log save failed: {error}");
                self.publish_component_state(
                    Component::Logging,
                    ComponentState::Failed,
                    format!("QSO log save failed: {error}"),
                );
                LogOutcome::Failed
            }
        }
    }

    pub(crate) fn append_qso(&mut self, mut record: QsoRecord, status: &str) {
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
        let published = self.qso_log.contacts.last().cloned();
        let persistence = self.persist_qso_log(status);
        if let Some(last) = &published {
            if persistence == LogOutcome::Persisted {
                self.app_events.publish(AppEvent::QsoLogged {
                    id: last.id,
                    schema_version: self.qso_log.version,
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
                    persistence,
                });
            }
        }
        self.qso_selected = self.qso_log.contacts.last().map(|contact| contact.id);
        if persistence != LogOutcome::Persisted {
            self.qso_log_dirty = true;
        }
        if let Some(record) = &published {
            self.publish_qso_to_server(record);
        }
    }
}
