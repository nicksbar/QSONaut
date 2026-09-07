use super::super::*;
use qsonaut_third_party::rade::RadeMode;

/// RADE is a digital-voice workspace, not a second logging workflow. The
/// normal Voice band vocabulary is useful while the modem is being integrated.
pub(crate) const BAND_PLAN: &[(&str, u64)] = super::voice::BAND_PLAN;

impl QsonautGuiApp {
    pub(crate) fn draw_rade_workspace(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let frequency = snapshot
            .frequency_hz
            .map(|hz| format!("{:.6} MHz", hz as f64 / 1_000_000.0))
            .unwrap_or_else(|| "RADIO OFFLINE".to_string());
        let band = snapshot
            .frequency_hz
            .map(band_for_frequency)
            .filter(|band| !band.is_empty())
            .unwrap_or("--");
        let capabilities = self.rade_mode.capabilities();

        egui::Frame::NONE
            .fill(Color32::from_rgb(10, 20, 32))
            .inner_margin(egui::Margin::same(14))
            .corner_radius(egui::CornerRadius::same(8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("RADE // DIGITAL VOICE")
                                .size(24.0)
                                .strong()
                                .color(Color32::from_rgb(117, 225, 255)),
                        );
                        ui.label(
                            RichText::new("A voice modem cockpit for the QSONaut signal path")
                                .color(theme_muted(ui)),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("{frequency}  ·  {band}"))
                                .monospace()
                                .strong(),
                        );
                    });
                });

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("WAVEFORM").strong().color(theme_muted(ui)));
                    for mode in [RadeMode::V1, RadeMode::V2] {
                        let selected = self.rade_mode == mode;
                        let label = if mode.development() {
                            "V2 · UPSTREAM"
                        } else {
                            "V1 · STABLE"
                        };
                        if ui
                            .add_sized(
                                [150.0, 30.0],
                                egui::Button::new(RichText::new(label).strong()).selected(selected),
                            )
                            .on_hover_text(mode.label())
                            .clicked()
                        {
                            self.rade_mode = mode;
                        }
                    }
                    ui.label(
                        RichText::new(capabilities.label)
                            .small()
                            .italics()
                            .color(if capabilities.development {
                                theme_warning(ui)
                            } else {
                                theme_success(ui)
                            }),
                    );
                });

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let tx_active = self.digital_tx_active.load(Ordering::Acquire);
                    rade_lane(
                        ui,
                        "RECEIVE",
                        "ADAPTER LIVE",
                        "Native RX routes RADE features through the third-party speech decoder when enabled.",
                        theme_success(ui),
                    );
                    rade_lane(
                        ui,
                        "TRANSMIT",
                        if tx_active {
                            "CAPTURING / TX"
                        } else if capabilities.supports_transmit {
                            "PUSH TO TALK"
                        } else {
                            "SAFE / UNAVAILABLE"
                        },
                        if tx_active {
                            "Voice capture or RADE waveform playback is active. Stop safely to release PTT."
                        } else {
                            "Capture a short voice burst, encode it through RADE, then transmit with the normal PTT safety boundary."
                        },
                        if tx_active {
                            theme_warning(ui)
                        } else {
                            theme_success(ui)
                        },
                    );
                });

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if self.digital_tx_active.load(Ordering::Acquire) {
                        if ui
                            .button("STOP RADE TX")
                            .on_hover_text("Cancel capture/playback and release PTT")
                            .clicked()
                        {
                            self.stop_native_digital_tx();
                        }
                    } else if capabilities.supports_transmit
                        && ui
                            .button("🎙 CAPTURE 4s + TRANSMIT")
                            .on_hover_text(
                                "Capture from the global voice microphone, encode through RADE, and transmit",
                            )
                            .clicked()
                    {
                        self.queue_rade_tx();
                    }
                    ui.label(
                        RichText::new(&self.digital_tx_status)
                            .small()
                            .color(theme_muted(ui)),
                    );
                });

                ui.add_space(12.0);
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("SIGNAL PATH").strong());
                        ui.label(
                            RichText::new("contract-first · no GUI codec hidden inside")
                                .small()
                                .color(theme_muted(ui)),
                        );
                    });
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        for (label, color) in [
                            ("MIC / PCM 16 kHz", Color32::from_rgb(120, 220, 255)),
                            ("VOICE CONTRACT", Color32::from_rgb(195, 160, 255)),
                            ("RADE MODEM 8 kHz", Color32::from_rgb(255, 190, 100)),
                            ("RADIO IQ / AUDIO", Color32::from_rgb(130, 235, 170)),
                        ] {
                            ui.label(
                                RichText::new(format!("  {label}  "))
                                    .monospace()
                                    .strong()
                                    .background_color(color.gamma_multiply(0.16))
                                    .color(color),
                            );
                            if label != "RADIO IQ / AUDIO" {
                                ui.label(RichText::new("›").size(20.0).color(theme_muted(ui)));
                            }
                        }
                    });
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(
                            "The adapter owns RADE V1/V2. QSONaut owns device choice, timing, operator intent, and the TX safety boundary.",
                        )
                        .small()
                        .color(theme_muted(ui)),
                    );
                });

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("MODEM CAPABILITIES")
                            .strong()
                            .color(theme_muted(ui)),
                    );
                    ui.label(format!(
                        "RX {}  ·  TX {}  ·  modem {} Hz  ·  speech {} Hz",
                        if capabilities.supports_receive {
                            "advertised"
                        } else {
                            "off"
                        },
                        if capabilities.supports_transmit {
                            "advertised"
                        } else {
                            "off"
                        },
                        capabilities.modem_input_rate_hz,
                        capabilities.speech_input_rate_hz,
                    ));
                });
                ui.label(
                    RichText::new(
                        "The adapter owns RADE V1/V2 and speech conversion; QSONaut owns capture, buffering, timing, and the conspicuous TX safety boundary.",
                    )
                    .small()
                    .color(theme_muted(ui)),
                );
            });
    }
}

impl QsonautGuiApp {
    pub(crate) fn queue_rade_tx(&mut self) {
        if self.ft8_tx_active.load(Ordering::Acquire)
            || self.digital_tx_active.load(Ordering::Acquire)
        {
            self.digital_tx_status = "RADE TX blocked: another transmission is active".into();
            return;
        }
        let Some(command_tx) = self.command_tx.clone() else {
            self.digital_tx_status = "RADE TX unavailable: radio control is disabled".into();
            return;
        };
        self.digital_tx_abort.store(false, Ordering::Release);
        self.digital_tx_active.store(true, Ordering::Release);
        self.digital_tx_started = None;
        self.digital_queued_tx_message = None;
        self.digital_tx_status = format!(
            "RADE {} · capturing 4 seconds from voice microphone",
            self.rade_mode.label()
        );
        let job = RadeTxJob {
            mode: self.rade_mode,
            input_device: self.config.audio.voice_input_device.clone(),
            output_device: effective_audio_output_device(
                &self.config.radio.backend,
                self.config.audio.output_device.clone(),
            ),
            ptt_tail: Duration::from_millis(self.ptt_tail_ms),
            abort: self.digital_tx_abort.clone(),
            active: self.digital_tx_active.clone(),
            command_tx,
            event_tx: self.digital_tx_event_tx.clone(),
            state: self.state.clone(),
            repaint_ctx: self.repaint_ctx.clone(),
        };
        thread::spawn(move || run_rade_tx_job(job));
    }
}

fn rade_lane(ui: &mut egui::Ui, title: &str, status: &str, detail: &str, color: Color32) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_width(250.0);
        ui.label(RichText::new(title).strong().color(color));
        ui.label(RichText::new(status).size(18.0).strong());
        ui.label(RichText::new(detail).small().color(theme_muted(ui)));
    });
}
