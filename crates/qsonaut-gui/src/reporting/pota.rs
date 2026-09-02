use super::super::*;

#[derive(Debug, Deserialize)]
pub(crate) struct PotaApiSpot {
    pub(crate) activator: Option<String>,
    pub(crate) reference: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) frequency: Option<String>,
    pub(crate) mode: Option<String>,
}

impl QsonautGuiApp {
    pub(crate) fn pump_pota_spots(&mut self) {
        if !self.pota_enabled {
            return;
        }
        if let Some(rx) = &self.pota_lookup_rx {
            match rx.try_recv() {
                Ok(spots) => {
                    match spots {
                        Ok(spots) => {
                            let activators = spots
                                .iter()
                                .map(|spot| spot.activator.as_str())
                                .collect::<HashSet<_>>()
                                .len();
                            info!(
                                spots = spots.len(),
                                activators,
                                elapsed_ms = self.pota_last_lookup.elapsed().as_millis() as u64,
                                "POTA activator spots refreshed"
                            );
                            self.pota_spots = spots;
                            self.pota_last_updated = Some(Instant::now());
                            self.pota_last_error = None;
                            self.pota_history.push_back((Instant::now(), activators));
                            while self.pota_history.len() > 60 {
                                self.pota_history.pop_front();
                            }
                        }
                        Err(error) => {
                            warn!(
                                error = %error,
                                elapsed_ms = self.pota_last_lookup.elapsed().as_millis() as u64,
                                "POTA activator spot lookup failed"
                            );
                            self.pota_last_error = Some(error);
                        }
                    }
                    self.pota_lookup_rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    let error = "POTA lookup worker disconnected before returning a result";
                    warn!(
                        elapsed_ms = self.pota_last_lookup.elapsed().as_millis() as u64,
                        "POTA activator spot lookup worker disconnected"
                    );
                    self.pota_last_error = Some(error.to_string());
                    self.pota_lookup_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.pota_lookup_rx.is_some()
            || self.pota_last_lookup.elapsed() < Duration::from_secs(30)
        {
            return;
        }
        self.pota_last_lookup = Instant::now();
        info!("POTA activator spot lookup started");
        let (tx, rx) = mpsc::channel();
        self.pota_lookup_rx = Some(rx);
        thread::spawn(move || {
            let result = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .map_err(|error| error.to_string())
                .and_then(|client| {
                    client
                        .get("https://api.pota.app/spot/activator")
                        .send()
                        .map_err(|error| error.to_string())
                })
                .and_then(|response| {
                    response
                        .error_for_status()
                        .map_err(|error| error.to_string())
                })
                .and_then(|response| {
                    response
                        .json::<Vec<PotaApiSpot>>()
                        .map_err(|error| error.to_string())
                })
                .map(|spots| {
                    spots
                        .into_iter()
                        .filter_map(|spot| {
                            Some(PotaSpot {
                                activator: spot.activator?.trim().to_ascii_uppercase(),
                                reference: spot.reference?.trim().to_string(),
                                name: spot.name?.trim().to_string(),
                                frequency_hz: spot.frequency?.parse::<f64>().ok()?.round() as u64
                                    * 1_000,
                                mode: spot.mode?.trim().to_ascii_uppercase(),
                            })
                        })
                        .collect()
                });
            let _ = tx.send(result);
        });
    }
}
