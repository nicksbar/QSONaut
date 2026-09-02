use super::super::*;

pub(crate) fn enrich_qso_from_hamdb(record: &mut QsoRecord, cache: &HamDbCache, now: u64) {
    let callsign = record.callsign.trim().to_ascii_uppercase();
    if callsign.is_empty() {
        return;
    }
    let cached = cache
        .get_fresh(&callsign, now, HAMDB_CACHE_TTL_SECONDS)
        .ok()
        .flatten();
    let Some(entry) = cached else {
        return;
    };
    if record.grid.trim().is_empty() {
        record.grid = entry.grid.clone();
    }
    if record.state.trim().is_empty() {
        record.state = entry.state.clone();
    }
    record.hamdb = Some(entry);
}

impl QsonautGuiApp {
    pub(crate) fn pump_hamdb_lookup(&mut self) {
        let Some(rx) = self.hamdb_lookup_rx.as_ref() else {
            return;
        };
        let entry = match rx.try_recv() {
            Ok(Some(entry)) => entry,
            Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                warn!(callsign = %self.voice_lookup_requested, "HamDB callsign lookup returned no record");
                self.voice_lookup_status = "HamDB: callsign not found".to_string();
                self.hamdb_lookup_rx = None;
                return;
            }
            Err(mpsc::TryRecvError::Empty) => return,
        };
        let cache = HamDbCache::open(&hamdb_cache_path()).ok();
        let voice_match = self
            .voice_lookup_requested
            .eq_ignore_ascii_case(&entry.callsign);
        if voice_match {
            info!(callsign = %entry.callsign, "HamDB Voice contact lookup completed");
            if self.voice_grid.trim().is_empty() {
                self.voice_grid = entry.grid.clone();
            }
            if self.voice_state.trim().is_empty() {
                self.voice_state = entry.state.clone();
            }
            self.voice_hamdb = Some(entry.clone());
            self.voice_lookup_status = "HamDB: operator found".to_string();
        }
        let mut log_updated = false;
        for record in self
            .qso_log
            .contacts
            .iter_mut()
            .filter(|record| record.callsign.eq_ignore_ascii_case(&entry.callsign))
        {
            log_updated = true;
            if record.grid.trim().is_empty() {
                record.grid = entry.grid.clone();
            }
            if record.state.trim().is_empty() {
                record.state = entry.state.clone();
            }
            record.hamdb = Some(entry.clone());
        }
        if let Some(cache) = cache {
            let _ = cache.upsert(&entry);
        }
        if log_updated {
            self.qso_log_dirty = true;
            self.persist_qso_log("HamDB details saved to");
        }
        self.hamdb_lookup_rx = None;
    }

    pub(crate) fn pump_hamdb_profile_lookup(&mut self) {
        let Some(rx) = self.hamdb_profile_lookup_rx.as_ref() else {
            return;
        };
        let entry = match rx.try_recv() {
            Ok(Some(entry)) => entry,
            Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                warn!(callsign = %self.station_callsign, "HamDB operator profile lookup returned no record");
                self.profile_io_status = "HamDB did not return a license record".to_string();
                self.hamdb_profile_lookup_rx = None;
                return;
            }
            Err(mpsc::TryRecvError::Empty) => return,
        };
        info!(callsign = %entry.callsign, "HamDB operator profile lookup completed");
        self.station_callsign = entry.callsign.clone();
        if !entry.grid.trim().is_empty() {
            self.station_grid = entry.grid.clone();
        }
        let qth = [
            entry.address_line_1.trim(),
            entry.address_line_2.trim(),
            entry.state.trim(),
            entry.country.trim(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
        if !qth.is_empty() {
            self.station_qth = qth;
        }
        self.config.station.callsign = Some(self.station_callsign.clone());
        self.config.station.grid =
            (!self.station_grid.trim().is_empty()).then(|| self.station_grid.clone());
        if let Ok(cache) = HamDbCache::open(&hamdb_cache_path()) {
            let _ = cache.upsert(&entry);
        }
        self.profile_dirty = true;
        self.persist_profile("Loaded license profile from HamDB");
        self.emit_operator_profile_hook("profile_loaded_from_hamdb");
        self.hamdb_profile_lookup_rx = None;
    }

    pub(crate) fn load_profile_from_hamdb(&mut self) {
        let callsign = self.station_callsign.trim().to_ascii_uppercase();
        if callsign.is_empty() || !is_probable_callsign(&callsign) {
            self.profile_io_status =
                "Enter a valid callsign before loading HamDB profile".to_string();
            return;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        info!(callsign = %callsign, "HamDB operator profile lookup started");
        self.hamdb_profile_lookup_rx = Some(spawn_hamdb_lookup(callsign, now));
        self.profile_io_status = "Loading license record from HamDB…".to_string();
    }

    pub(crate) fn refresh_hamdb_for_contact(&mut self, index: usize) {
        let Some(record) = self.qso_log.contacts.get(index) else {
            return;
        };
        let callsign = record.callsign.trim().to_ascii_uppercase();
        if callsign.is_empty() {
            self.qso_log_status = "HamDB lookup requires a callsign".to_string();
            return;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        self.hamdb_lookup_rx = Some(spawn_hamdb_lookup(callsign, now));
        self.qso_log_status = "Refreshing HamDB details…".to_string();
    }
}

#[derive(Debug, Deserialize)]
struct HamDbResponse {
    hamdb: HamDbPayload,
}

#[derive(Debug, Deserialize)]
struct HamDbPayload {
    callsign: HamDbCallsign,
}

#[derive(Debug, Deserialize)]
struct HamDbCallsign {
    call: String,
    #[serde(default)]
    class: String,
    #[serde(default)]
    expires: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    grid: String,
    #[serde(default, alias = "lat")]
    latitude: String,
    #[serde(default, alias = "lon")]
    longitude: String,
    #[serde(default, alias = "fname")]
    first_name: String,
    #[serde(default, alias = "mi")]
    middle_name: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    suffix: String,
    #[serde(default, alias = "addr1")]
    address_line_1: String,
    #[serde(default, alias = "addr2")]
    address_line_2: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    zip: String,
    #[serde(default)]
    country: String,
}

pub(crate) fn spawn_hamdb_lookup(
    callsign: String,
    completed_at_unix: u64,
) -> mpsc::Receiver<Option<HamDbCacheEntry>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .ok()
            .and_then(|client| {
                client
                    .get(format!(
                        "https://api.hamdb.org/{}/json/QSONaut",
                        callsign.trim()
                    ))
                    .send()
                    .ok()
            })
            .filter(|response| response.status().is_success())
            .and_then(|response| response.json::<HamDbResponse>().ok())
            .map(|response| HamDbCacheEntry {
                callsign: response.hamdb.callsign.call.trim().to_ascii_uppercase(),
                class: response.hamdb.callsign.class.trim().to_string(),
                expires: response.hamdb.callsign.expires.trim().to_string(),
                status: response.hamdb.callsign.status.trim().to_string(),
                grid: response.hamdb.callsign.grid.trim().to_ascii_uppercase(),
                latitude: response.hamdb.callsign.latitude.trim().to_string(),
                longitude: response.hamdb.callsign.longitude.trim().to_string(),
                first_name: response.hamdb.callsign.first_name.trim().to_string(),
                middle_name: response.hamdb.callsign.middle_name.trim().to_string(),
                name: response.hamdb.callsign.name.trim().to_string(),
                suffix: response.hamdb.callsign.suffix.trim().to_string(),
                address_line_1: response.hamdb.callsign.address_line_1.trim().to_string(),
                address_line_2: response.hamdb.callsign.address_line_2.trim().to_string(),
                state: response.hamdb.callsign.state.trim().to_ascii_uppercase(),
                zip: response.hamdb.callsign.zip.trim().to_string(),
                country: response.hamdb.callsign.country.trim().to_string(),
                fetched_at_unix: completed_at_unix,
            });
        let _ = tx.send(result);
    });
    rx
}
