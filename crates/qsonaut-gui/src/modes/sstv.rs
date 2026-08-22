use super::super::*;
use image::imageops::FilterType;

pub(crate) const BAND_PLAN: &[(&str, u64)] = &[
    ("80m", 3_845_000),
    ("40m", 7_171_000),
    ("20m", 14_230_000),
    ("15m", 21_340_000),
    ("10m", 28_680_000),
];

fn fitted_sstv_body_height(available_height: f32) -> f32 {
    const TX_SAFETY_HEIGHT: f32 = 78.0;
    (available_height - TX_SAFETY_HEIGHT).max(0.0)
}

impl QsonautGuiApp {
    pub(crate) fn draw_sstv_workspace(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        _snapshot: &GuiState,
    ) {
        self.poll_local_image_events();
        self.sstv_file_dialog.update(ctx);
        if let Some(path) = self.sstv_file_dialog.take_picked() {
            self.sstv_image_path = path.display().to_string();
            match std::fs::read(&path) {
                Ok(bytes) => self.install_sstv_image(&bytes, "Loaded image"),
                Err(error) => self.local_image_status = format!("Image load failed: {error}"),
            }
        }
        let snapshot = self.state.lock().expect("ui state lock poisoned").clone();
        if snapshot.sstv_revision != self.sstv_texture_revision
            && snapshot.sstv_rgb.len() == qsonaut_sstv::WIDTH * qsonaut_sstv::HEIGHT * 3
        {
            let image = ColorImage::from_rgb(
                [qsonaut_sstv::WIDTH, qsonaut_sstv::HEIGHT],
                &snapshot.sstv_rgb,
            );
            self.sstv_texture = Some(ctx.load_texture("sstv-frame", image, TextureOptions::LINEAR));
            self.sstv_texture_revision = snapshot.sstv_revision;
        }

        ui.horizontal_wrapped(|ui| {
            ui.heading(format!("📺 SSTV · {}", self.sstv_tx_mode.name()));
            ui.separator();
            ui.label(RichText::new("RX AUTO (VIS)").color(theme_success(ui)));
            ui.separator();
            ui.label(format!(
                "Detected: {}",
                snapshot
                    .sstv_detected_mode
                    .map(|mode| mode.name())
                    .unwrap_or("waiting for header")
            ));
            ui.separator();
            ui.label(format!(
                "Radio {:.3} MHz · USB FIL1",
                snapshot.frequency_hz.unwrap_or_default() as f64 / 1_000_000.0
            ));
        });
        ui.label(
            RichText::new("Calling frequencies are voluntary band-plan centers; confirm your license privileges and a clear channel before transmitting.")
                .small()
                .color(theme_warning(ui)),
        );
        ui.label(
            RichText::new(
                "RX auto-detects the VIS header and names Martin, Scottie, Robot, and PD modes. Live image reconstruction is currently Martin M1; other modes are identified while the multi-mode streaming adapter is validated. Click the signal center in the audio waterfall to align the decoder.",
            )
            .small()
            .color(theme_accent(ui)),
        );
        ui.add_space(4.0);

        // Reserve the safety strip and make the two work areas own only the
        // remaining viewport. Each column then manages its own overflow rather
        // than making the entire SSTV workspace taller than the central panel.
        let body_height = fitted_sstv_body_height(ui.available_height());
        ui.columns(2, |columns| {
            let (left, right) = columns.split_at_mut(1);
            let left = &mut left[0];
            let right = &mut right[0];

            left.allocate_ui_with_layout(
                egui::vec2(left.available_width(), body_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| egui::Frame::dark_canvas(ui.style()).show(ui, |ui| {
                ui.label(RichText::new("SSTV FRAME").strong());
                ui.horizontal_wrapped(|ui| {
                    if ui.button("📂 OPEN IMAGE…").clicked() {
                        self.sstv_file_dialog.pick_file();
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut self.sstv_image_path)
                            .hint_text("PNG/JPEG path")
                            .desired_width((ui.available_width() - 92.0).max(80.0)),
                    );
                    if ui.small_button("LOAD PATH").clicked() {
                        match std::fs::read(self.sstv_image_path.trim()) {
                            Ok(bytes) => self.install_sstv_image(&bytes, "Loaded image"),
                            Err(error) => {
                                self.local_image_status = format!("Image load failed: {error}")
                            }
                        }
                    }
                });
                ui.label(RichText::new(&snapshot.sstv_status).monospace());
                if let Some(progress) = snapshot.sstv_progress {
                    ui.add(
                        egui::ProgressBar::new(progress)
                            .desired_width(ui.available_width())
                            .text(format!("Receiving {:.0}%", progress * 100.0)),
                    );
                }
                    let available_width = ui.available_width().max(1.0);
                    let available_height = ui.available_height().max(1.0);
                let image_width = available_width.min(available_height / 0.8);
                let size = egui::vec2(image_width, image_width * 0.8);
                if let Some(texture) = &self.sstv_texture {
                    ui.add(egui::Image::new((texture.id(), size)).corner_radius(5.0));
                } else {
                    ui.allocate_ui_with_layout(
                        size,
                        egui::Layout::centered_and_justified(egui::Direction::TopDown),
                        |ui| {
                            ui.label(
                                RichText::new("Listening for a frame\nor load/generate one for TX")
                                    .color(theme_muted(ui)),
                            );
                        },
                    );
                }
                }),
            );

            egui::ScrollArea::vertical()
                .id_salt("sstv_local_image_lab_scroll")
                .max_height(body_height)
                .auto_shrink([false, false])
                .show(right, |ui| egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("TX mode").strong());
                    egui::ComboBox::from_id_salt("sstv_tx_mode")
                        .selected_text(self.sstv_tx_mode.name())
                        .show_ui(ui, |ui| {
                            for &mode in qsonaut_sstv::supported_modes() {
                                let (width, height) = mode.resolution();
                                ui.selectable_value(
                                    &mut self.sstv_tx_mode,
                                    mode,
                                    format!(
                                        "{} · {}×{} · {:.0}s",
                                        mode.name(),
                                        width,
                                        height,
                                        qsonaut_sstv::mode_duration_seconds(mode),
                                    ),
                                );
                            }
                        });
                    ui.label(
                        RichText::new("RX: Auto (VIS)")
                            .small()
                            .color(theme_accent(ui)),
                    );
                });
                ui.separator();
                ui.label(RichText::new("🧠 LOCAL IMAGE LAB").strong());
                ui.label(
                    RichText::new("Hard local-only policy: HTTP requests are blocked unless the host is localhost or a loopback IP.")
                        .small()
                        .color(theme_accent(ui)),
                );
                let old_provider = self.local_image_settings.provider;
                ui.horizontal(|ui| {
                    ui.label("Server");
                    egui::ComboBox::from_id_salt("sstv-local-provider")
                        .selected_text(self.local_image_settings.provider.label())
                        .show_ui(ui, |ui| {
                            for provider in LocalImageProvider::ALL {
                                ui.selectable_value(
                                    &mut self.local_image_settings.provider,
                                    provider,
                                    provider.label(),
                                );
                            }
                        });
                });
                if old_provider != self.local_image_settings.provider {
                    self.local_image_models.clear();
                    self.local_image_settings.model.clear();
                }
                let find_models = ui.horizontal(|ui| {
                    ui.label("URL");
                    match self.local_image_settings.provider {
                        LocalImageProvider::Ollama => {
                            ui.add(egui::TextEdit::singleline(&mut self.local_image_settings.ollama_url).desired_width(220.0));
                        }
                        LocalImageProvider::Lemonade => {
                            ui.add(egui::TextEdit::singleline(&mut self.local_image_settings.lemonade_url).desired_width(220.0));
                        }
                    }
                    ui.button("Find models").clicked()
                }).inner;
                if find_models {
                    self.refresh_local_image_models();
                }
                ui.horizontal(|ui| {
                    ui.label("Model");
                    egui::ComboBox::from_id_salt("sstv-local-model")
                        .selected_text(if self.local_image_settings.model.is_empty() {
                            "Select a local model"
                        } else {
                            &self.local_image_settings.model
                        })
                        .show_ui(ui, |ui| {
                            for model in &self.local_image_models {
                                ui.selectable_value(
                                    &mut self.local_image_settings.model,
                                    model.clone(),
                                    model,
                                );
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label("Output");
                    ui.add(egui::DragValue::new(&mut self.local_image_settings.width).range(256..=2048));
                    ui.label("×");
                    ui.add(egui::DragValue::new(&mut self.local_image_settings.height).range(256..=2048));
                    ui.separator();
                    ui.label("Steps");
                    ui.add(egui::DragValue::new(&mut self.local_image_settings.steps).range(1..=100));
                });
                if self.sstv_ai_prompt.is_empty() {
                    self.sstv_ai_prompt = self.sstv_activity_prompt(&snapshot);
                }
                ui.add(
                    egui::TextEdit::multiline(&mut self.sstv_ai_prompt)
                        .desired_rows(5)
                        .hint_text("Describe the image to transmit"),
                );
                ui.horizontal(|ui| {
                    if ui.button("Use current activity").clicked() {
                        self.sstv_ai_prompt = self.sstv_activity_prompt(&snapshot);
                    }
                    if ui
                        .add_enabled(
                            !self.local_image_settings.model.is_empty(),
                            egui::Button::new("✨ GENERATE LOCALLY"),
                        )
                        .clicked()
                    {
                        self.generate_local_sstv_image();
                    }
                });
                ui.label(RichText::new(&self.local_image_status).small().color(theme_muted(ui)));
                }));
        });

        ui.add_space(5.0);
        let has_frame = snapshot.sstv_rgb.len() == qsonaut_sstv::WIDTH * qsonaut_sstv::HEIGHT * 3;
        egui::Frame::group(ui.style())
            .fill(if self.sstv_tx_armed {
                Color32::from_rgb(76, 31, 25)
            } else {
                Color32::from_rgb(20, 43, 52)
            })
            .stroke(egui::Stroke::new(
                2.0,
                if self.sstv_tx_armed {
                    Color32::LIGHT_RED
                } else {
                    Color32::LIGHT_BLUE
                },
            ))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            has_frame && !self.digital_tx_active.load(Ordering::Acquire),
                            egui::Button::new(if self.sstv_tx_armed {
                                "🔓 SSTV TX ARMED"
                            } else {
                                "🔐 ARM SSTV TX"
                            }),
                        )
                        .clicked()
                    {
                        self.sstv_tx_armed = !self.sstv_tx_armed;
                    }
                    if ui
                        .add_enabled(
                            self.sstv_tx_armed
                                && has_frame
                                && !self.digital_tx_active.load(Ordering::Acquire),
                            egui::Button::new(format!(
                                "🔥 TRANSMIT {} · ~{:.0}s",
                                self.sstv_tx_mode.name(),
                                qsonaut_sstv::mode_duration_seconds(self.sstv_tx_mode),
                            ))
                            .fill(Color32::from_rgb(145, 42, 34)),
                        )
                        .clicked()
                    {
                        self.start_sstv_tx(self.sstv_tx_mode, &snapshot.sstv_rgb);
                    }
                    if ui
                        .add_enabled(
                            self.digital_tx_active.load(Ordering::Acquire),
                            egui::Button::new("⛔ STOP SSTV TX"),
                        )
                        .clicked()
                    {
                        self.sstv_tx_armed = false;
                        self.stop_native_digital_tx();
                    }
                    ui.label(RichText::new(&self.digital_tx_status).strong());
                });
            });
    }

    fn sstv_activity_prompt(&self, snapshot: &GuiState) -> String {
        format!(
            "Create bold, high-contrast amateur radio SSTV QSL artwork for callsign {} in {} {}. Current activity: {:.3} MHz SSTV {}. Use a striking radio-space aesthetic, one strong central subject, large readable callsign, no tiny text, and a composition that survives analog SSTV transmission.",
            self.station_callsign_or_default(),
            self.station_qth.trim(),
            self.station_grid_or_default(),
            snapshot.frequency_hz.unwrap_or(14_230_000) as f64 / 1_000_000.0,
            self.sstv_tx_mode.name(),
        )
    }

    fn refresh_local_image_models(&mut self) {
        if let Err(error) =
            local_ai::validate_loopback_endpoint(self.local_image_settings.endpoint())
        {
            self.local_image_status = error.to_string();
            return;
        }
        let settings = self.local_image_settings.clone();
        let sender = self.local_image_event_tx.clone();
        self.local_image_status = format!("Checking {}…", settings.provider.label());
        thread::spawn(move || {
            let result = local_ai::list_models(&settings).map_err(|error| error.to_string());
            let _ = sender.send(LocalImageEvent::Models(result));
        });
    }

    fn generate_local_sstv_image(&mut self) {
        if let Err(error) =
            local_ai::validate_loopback_endpoint(self.local_image_settings.endpoint())
        {
            self.local_image_status = error.to_string();
            return;
        }
        let _ = self.local_image_settings.save();
        let settings = self.local_image_settings.clone();
        let prompt = self.sstv_ai_prompt.clone();
        let sender = self.local_image_event_tx.clone();
        self.local_image_status = format!(
            "{} is generating with {}… this can take several minutes",
            settings.provider.label(),
            settings.model
        );
        thread::spawn(move || {
            let result = local_ai::generate(&settings, &prompt).map_err(|error| error.to_string());
            let _ = sender.send(LocalImageEvent::Generated(result));
        });
    }

    fn poll_local_image_events(&mut self) {
        while let Ok(event) = self.local_image_event_rx.try_recv() {
            match event {
                LocalImageEvent::Models(Ok(models)) => {
                    self.local_image_models = models;
                    if self.local_image_settings.model.is_empty() {
                        if let Some(model) = self.local_image_models.first() {
                            self.local_image_settings.model = model.clone();
                        }
                    }
                    self.local_image_status = format!(
                        "Found {} local model{}",
                        self.local_image_models.len(),
                        if self.local_image_models.len() == 1 {
                            ""
                        } else {
                            "s"
                        }
                    );
                    let _ = self.local_image_settings.save();
                }
                LocalImageEvent::Models(Err(error)) | LocalImageEvent::Generated(Err(error)) => {
                    self.local_image_status = error;
                }
                LocalImageEvent::Generated(Ok(bytes)) => {
                    self.install_sstv_image(&bytes, "Generated locally");
                }
            }
        }
    }

    fn install_sstv_image(&mut self, bytes: &[u8], source: &str) {
        match image::load_from_memory(bytes) {
            Ok(image) => {
                let rgb = image
                    .resize_to_fill(
                        qsonaut_sstv::WIDTH as u32,
                        qsonaut_sstv::HEIGHT as u32,
                        FilterType::Lanczos3,
                    )
                    .to_rgb8();
                let generated_dir = app_config_dir().join("sstv-images");
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_secs())
                    .unwrap_or_default();
                let saved = std::fs::create_dir_all(&generated_dir).and_then(|_| {
                    rgb.save(generated_dir.join(format!("sstv-{timestamp}.png")))
                        .map_err(std::io::Error::other)
                });
                let mut shared = self.state.lock().expect("ui state lock poisoned");
                shared.sstv_rgb = rgb.into_raw();
                shared.sstv_revision = shared.sstv_revision.wrapping_add(1);
                shared.sstv_status = format!("{source}: ready for SSTV TX");
                self.local_image_status = match saved {
                    Ok(()) => format!("{source}; 320×256 SSTV frame saved locally"),
                    Err(error) => format!("{source}; local save failed: {error}"),
                };
            }
            Err(error) => self.local_image_status = format!("Image decode failed: {error}"),
        }
    }

    fn start_sstv_tx(&mut self, mode: qsonaut_sstv::SstvMode, rgb: &[u8]) {
        if self.ft8_tx_active.load(Ordering::Acquire)
            || self.digital_tx_active.load(Ordering::Acquire)
        {
            self.digital_tx_status = "SSTV TX blocked: another transmission is active".to_string();
            return;
        }
        let Some(command_tx) = self.command_tx.clone() else {
            self.digital_tx_status = "SSTV TX unavailable: radio control is disabled".to_string();
            return;
        };
        match qsonaut_sstv::encode_rgb_mode_12k(
            mode,
            qsonaut_sstv::WIDTH as u32,
            qsonaut_sstv::HEIGHT as u32,
            rgb,
        ) {
            Ok(pcm) => {
                let now_s = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_secs_f64())
                    .unwrap_or_default();
                let audio_start_s = now_s + self.ptt_lead_ms as f64 / 1_000.0 + 0.05;
                self.digital_tx_abort.store(false, Ordering::Release);
                self.digital_tx_active.store(true, Ordering::Release);
                self.digital_queued_tx_message = Some(format!("{} image", mode.name()));
                self.digital_tx_status =
                    format!("🔥 SSTV queued; keying radio for {}", mode.name());
                self.sstv_tx_armed = false;
                let job = DigitalTxJob {
                    mode: WorkspaceMode::Sstv,
                    period: now_s.floor() as u64,
                    slot_seconds: 1.0,
                    audio_offset_s: 0.0,
                    audio_start_s: Some(audio_start_s),
                    pcm: Arc::new(pcm),
                    ptt_lead: Duration::from_millis(self.ptt_lead_ms),
                    ptt_tail: Duration::from_millis(self.ptt_tail_ms),
                    output_device: self.config.audio.output_device.clone(),
                    abort: self.digital_tx_abort.clone(),
                    active: self.digital_tx_active.clone(),
                    command_tx,
                    event_tx: self.digital_tx_event_tx.clone(),
                    state: self.state.clone(),
                    repaint_ctx: self.repaint_ctx.clone(),
                };
                thread::spawn(move || run_digital_tx_job(job));
            }
            Err(error) => self.digital_tx_status = format!("SSTV encode failed: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fitted_sstv_body_height;

    #[test]
    fn sstv_body_tracks_the_available_viewport() {
        assert_eq!(fitted_sstv_body_height(500.0), 422.0);
        assert_eq!(fitted_sstv_body_height(200.0), 122.0);
        assert_eq!(fitted_sstv_body_height(60.0), 0.0);
    }
}
