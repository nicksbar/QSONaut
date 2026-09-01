use super::super::*;

impl QsonautGuiApp {
    pub(in super::super) fn test_cat_connection(&mut self) {
        let backend = self.config.radio.backend.clone();
        let model = self.config.radio.model.clone();
        let port = self.config.radio.serial_port.clone().unwrap_or_default();
        let baud_rate = self.config.radio.baud_rate;
        let controller_civ_address = self.config.radio.controller_civ_address;
        let radio_civ_address = self.config.radio.civ_address;
        let (tx, rx) = mpsc::channel();

        self.cat_test_status = None;
        self.cat_test_rx = Some(rx);
        info!(
            backend = %backend,
            model = %model,
            port = %if port.is_empty() { "auto" } else { &port },
            baud = baud_rate,
            "CAT connection test requested"
        );

        thread::spawn(move || {
            let result = Self::cat_connection_result(
                &backend,
                &model,
                &port,
                baud_rate,
                controller_civ_address,
                radio_civ_address,
            );

            match &result {
                Ok(message) => info!(
                    model = %model,
                    port = %port,
                    baud = baud_rate,
                    result = %message,
                    "CAT connection test succeeded"
                ),
                Err(error) => warn!(
                    model = %model,
                    port = %port,
                    baud = baud_rate,
                    error = %error,
                    "CAT connection test failed"
                ),
            }
            let _ = tx.send(result);
        });
    }

    pub(in super::super) fn cat_connection_result(
        backend: &str,
        model: &str,
        port: &str,
        baud_rate: u32,
        controller_civ_address: u8,
        radio_civ_address: u8,
    ) -> Result<String, String> {
        if !backend.eq_ignore_ascii_case("native") {
            Err(format!(
                "CAT test requires the Native Rigwright backend; selected backend is '{}'.",
                backend
            ))
        } else if port.is_empty() {
            Err(
                "CAT test requires a specific serial port; select a radio USB/serial port first."
                    .into(),
            )
        } else {
            match open_model_with_radio_address(
                model,
                port,
                baud_rate,
                controller_civ_address,
                Some(radio_civ_address),
            ) {
                Ok(radio) => match radio {
                    ConfiguredRadio::Yaesu(yaesu) => match yaesu.verify_model() {
                        Ok(()) => Ok(format!(
                            "CAT OK: {} answered and matched the selected profile at {} baud.",
                            model, baud_rate
                        )),
                        Err(error) => Err(format!(
                            "CAT probe failed for {} at {} baud: {error}",
                            model, baud_rate
                        )),
                    },
                    ConfiguredRadio::Kenwood(kenwood) => match kenwood.verify_model() {
                        Ok(()) => Ok(format!(
                            "CAT OK: {} answered and matched the selected profile at {} baud.",
                            model, baud_rate
                        )),
                        Err(error) => Err(format!(
                            "CAT probe failed for {} at {} baud: {error}",
                            model, baud_rate
                        )),
                    },
                    ConfiguredRadio::Icom(icom) => {
                        match tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                        {
                            Ok(runtime) => match runtime.block_on(Radio::get_frequency_hz(&icom)) {
                                Ok(frequency_hz) => Ok(format!(
                                    "CAT OK: {} answered at {} baud (frequency {} Hz).",
                                    model, baud_rate, frequency_hz
                                )),
                                Err(error) => Err(format!(
                                    "CAT probe failed for {} at {} baud: {error}",
                                    model, baud_rate
                                )),
                            },
                            Err(error) => Err(format!("CAT test runtime failed: {error}")),
                        }
                    }
                    _ => Err(format!(
                        "CAT testing is not implemented for the selected native profile '{}'.",
                        model
                    )),
                },
                Err(error) => Err(format!(
                    "Could not open CAT port '{}' at {} baud for {}: {error}",
                    port, baud_rate, model
                )),
            }
        }
    }
}
