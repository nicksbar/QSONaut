use super::super::*;
use image::imageops::FilterType;
use imageproc::drawing::draw_text_mut;
use imageproc::pixelops::weighted_sum;
use ab_glyph::PxScale;
use std::io::Cursor;

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

fn load_overlay_font() -> Option<ab_glyph::FontArc> {
    [
        r"C:\Windows\Fonts\arialbd.ttf",
        r"C:\Windows\Fonts\segoeuib.ttf",
        r"C:\Windows\Fonts\arial.ttf",
    ]
    .iter()
    .find_map(|path| std::fs::read(path).ok())
    .and_then(|bytes| ab_glyph::FontArc::try_from_vec(bytes).ok())
}

impl QsonautGuiApp {
    fn reset_sstv_overlay(&mut self) {
        self.sstv_overlay_callsign = true;
        self.sstv_overlay_grid = true;
        self.sstv_overlay_frequency = false;
        self.sstv_overlay_mode = true;
        self.sstv_overlay_corner = SstvOverlayCorner::BottomLeft;
        self.sstv_overlay_background = Color32::BLACK;
        self.sstv_overlay_background_opacity = 0.65;
        self.sstv_overlay_revision = self.sstv_overlay_revision.wrapping_add(1);
        self.rebuild_sstv_tx_image();
    }

    fn rebuild_sstv_tx_image(&mut self) {
        if self.sstv_tx_base_rgb.len() != self.sstv_tx_width * self.sstv_tx_height * 3 {
            return;
        }
        let Some(font) = load_overlay_font() else {
            self.local_image_status = "SSTV overlay font unavailable; install DejaVu Sans or Arial".to_string();
            return;
        };
        let base_image = match image::RgbImage::from_raw(
            self.sstv_tx_width as u32,
            self.sstv_tx_height as u32,
            self.sstv_tx_base_rgb.clone(),
        ) {
            Some(image) => image,
            None => return,
        };
        let mut image = image::RgbImage::from_pixel(
            self.sstv_tx_width as u32,
            self.sstv_tx_height as u32,
            image::Rgb([0, 0, 0]),
        );
        let zoom = self.sstv_background_zoom.clamp(0.25, 3.0);
        let center_x = (self.sstv_tx_width as f32 - 1.0) * 0.5;
        let center_y = (self.sstv_tx_height as f32 - 1.0) * 0.5;
        for y in 0..self.sstv_tx_height {
            for x in 0..self.sstv_tx_width {
                let source_x = ((x as f32 - center_x - self.sstv_background_pan_x) / zoom
                    + center_x)
                    .round() as i32;
                let source_y = ((y as f32 - center_y - self.sstv_background_pan_y) / zoom
                    + center_y)
                    .round() as i32;
                if source_x >= 0
                    && source_x < self.sstv_tx_width as i32
                    && source_y >= 0
                    && source_y < self.sstv_tx_height as i32
                {
                    *image.get_pixel_mut(x as u32, y as u32) =
                        *base_image.get_pixel(source_x as u32, source_y as u32);
                }
            }
        }
        let mut lines = Vec::new();
        if self.sstv_overlay_callsign && !self.station_callsign.trim().is_empty() {
            lines.push(self.station_callsign.trim().to_string());
        }
        if self.sstv_overlay_grid && !self.station_grid.trim().is_empty() {
            lines.push(self.station_grid.trim().to_string());
        }
        if self.sstv_overlay_frequency {
            let snapshot = self.state.lock().expect("ui state lock poisoned");
            if let Some(frequency_hz) = snapshot.frequency_hz {
                lines.push(format!("{:.3} MHz", frequency_hz as f32 / 1_000_000.0));
            }
        }
        if self.sstv_overlay_mode {
            lines.push(self.sstv_tx_mode.name().to_string());
        }
        let scale = PxScale::from((self.sstv_tx_height as f32 * 0.075).clamp(12.0, 24.0));
        let line_height = (scale.y * 1.1) as i32;
        let padding = 6_i32;
        let box_height = line_height * lines.len() as i32 + padding * 2;
        if !lines.is_empty() {
            let box_y = match self.sstv_overlay_corner {
                SstvOverlayCorner::TopLeft | SstvOverlayCorner::TopRight => padding,
                SstvOverlayCorner::BottomLeft | SstvOverlayCorner::BottomRight => {
                    self.sstv_tx_height as i32 - box_height - padding
                }
            }
            .clamp(0, self.sstv_tx_height as i32 - 1);
                let text_widths = lines
                    .iter()
                    .map(|line| line.len() as i32 * (scale.x as i32 / 2).max(1))
                    .max()
                    .unwrap_or(0);
                let box_width = (text_widths + padding * 2).clamp(1, self.sstv_tx_width as i32);
                let box_x = match self.sstv_overlay_corner {
                SstvOverlayCorner::TopLeft | SstvOverlayCorner::BottomLeft => padding,
                SstvOverlayCorner::TopRight | SstvOverlayCorner::BottomRight => {
                        self.sstv_tx_width as i32 - box_width - padding
                }
            }
            .clamp(0, self.sstv_tx_width as i32 - 1);
                let overlay_color = image::Rgb([
                    self.sstv_overlay_background.r(),
                    self.sstv_overlay_background.g(),
                    self.sstv_overlay_background.b(),
                ]);
                let opacity = self.sstv_overlay_background_opacity.clamp(0.0, 1.0);
                for y in box_y..(box_y + box_height).min(self.sstv_tx_height as i32) {
                    for x in box_x..(box_x + box_width).min(self.sstv_tx_width as i32) {
                    let pixel = image.get_pixel_mut(x as u32, y as u32);
                        *pixel = weighted_sum(*pixel, overlay_color, 1.0 - opacity, opacity);
                }
            }
            for (index, line) in lines.iter().enumerate() {
                let text_width = line.len() as i32 * (scale.x as i32 / 2).max(1);
                let text_x = match self.sstv_overlay_corner {
                    SstvOverlayCorner::TopLeft | SstvOverlayCorner::BottomLeft => box_x,
                    SstvOverlayCorner::TopRight | SstvOverlayCorner::BottomRight => {
                            (box_x + box_width - text_width - padding).max(box_x)
                    }
                };
                draw_text_mut(
                    &mut image,
                    image::Rgb([255, 255, 255]),
                    text_x,
                    box_y + padding + index as i32 * line_height,
                    scale,
                    &font,
                    line,
                );
            }
        }
        self.sstv_tx_rgb = image.into_raw();
        self.sstv_tx_revision = self.sstv_tx_revision.wrapping_add(1);
    }

    fn load_prior_received_sstv_images(&mut self) {
        let directory = qsonaut_log::app_config_dir().join("sstv-images");
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                tracing::warn!(path = %directory.display(), error = %error, "failed to scan prior SSTV images");
                return;
            }
        };
        let mut shared = self.state.lock().expect("ui state lock poisoned");
        let mut added = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("png")
                || !path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.starts_with("rx-") && name.ends_with(".png"))
            {
                continue;
            }
            let id = path.display().to_string();
            if shared
                .sstv_received_images
                .iter()
                .any(|image| image.id == id)
            {
                continue;
            }
            let decoded = match image::open(&path) {
                Ok(image) => image.to_rgb8(),
                Err(error) => {
                    tracing::warn!(path = %path.display(), error = %error, "failed to load prior SSTV image");
                    continue;
                }
            };
            let metadata = entry.metadata().ok();
            let received_unix_ms = metadata
                .and_then(|value| value.modified().ok())
                .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|value| value.as_millis())
                .unwrap_or_default();
            shared.sstv_received_images.push_back(ReceivedSstvImage {
                id: id.clone(),
                path: Some(id.clone()),
                mode: None,
                frequency_hz: None,
                width: decoded.width() as usize,
                height: decoded.height() as usize,
                rgb: decoded.into_raw(),
                received_unix_ms,
                analysis: None,
                debug_audio_path: None,
                debug_metadata_path: None,
            });
            added += 1;
        }
        if added > 0 {
            shared.sstv_received_revision = shared.sstv_received_revision.wrapping_add(1);
        }
        tracing::info!(added, total = shared.sstv_received_images.len(), "prior SSTV images loaded");
    }

    fn selected_received_sstv_image<'a>(
        &self,
        snapshot: &'a GuiState,
    ) -> Option<&'a ReceivedSstvImage> {
        self.sstv_active_received_id
            .as_deref()
            .and_then(|id| {
                snapshot
                    .sstv_received_images
                    .iter()
                    .find(|image| image.id == id)
            })
            .or_else(|| snapshot.sstv_received_images.front())
    }

    fn selected_received_sstv_png(&self, snapshot: &GuiState) -> Result<(String, Vec<u8>)> {
        let image = self
            .selected_received_sstv_image(snapshot)
            .ok_or_else(|| anyhow!("no received SSTV image selected"))?;
        let rgb =
            image::RgbImage::from_raw(image.width as u32, image.height as u32, image.rgb.clone())
                .ok_or_else(|| {
                anyhow!("selected received SSTV image dimensions do not match RGB data")
            })?;
        let mut cursor = Cursor::new(Vec::new());
        rgb.write_to(&mut cursor, image::ImageFormat::Png)
            .context("failed to encode received SSTV image")?;
        Ok((image.id.clone(), cursor.into_inner()))
    }

    fn update_received_sstv_textures(&mut self, ctx: &egui::Context, snapshot: &GuiState) {
        if self.sstv_received_texture_revision == snapshot.sstv_received_revision {
            return;
        }
        self.sstv_received_textures.clear();
        for image in &snapshot.sstv_received_images {
            if image.width == 0
                || image.height == 0
                || image.rgb.len() != image.width * image.height * 3
            {
                continue;
            }
            let color = ColorImage::from_rgb([image.width, image.height], &image.rgb);
            self.sstv_received_textures.insert(
                image.id.clone(),
                ctx.load_texture(
                    format!("sstv-rx-history-{}", image.id),
                    color,
                    TextureOptions::LINEAR,
                ),
            );
        }
        self.sstv_received_texture_revision = snapshot.sstv_received_revision;
    }

    fn selected_model_id_for_role(&self, role: LocalModelRole) -> &str {
        match role {
            LocalModelRole::Vision => &self.local_image_settings.vision_model,
            LocalModelRole::Image => &self.local_image_settings.image_model,
            LocalModelRole::Edit => &self.local_image_settings.edit_model,
        }
    }

    fn selected_model_for_role(&self, role: LocalModelRole) -> Result<String> {
        let selected = self.selected_model_id_for_role(role);
        let model = local_ai::model_for_role(&self.local_image_models, selected, role)?;
        tracing::info!(
            provider = %model.provider.label(),
            role = %role.label(),
            selected_model = %model.id,
            capabilities = %model.capabilities.summary(),
            "validated local AI model selection"
        );
        Ok(selected.trim().to_string())
    }

    fn draw_received_sstv_carousel(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Received images").strong());
            let count = snapshot.sstv_received_images.len();
            ui.label(
                RichText::new(format!(
                    "{count} frame{}",
                    if count == 1 { "" } else { "s" }
                ))
                .small()
                .color(theme_muted(ui)),
            );
            let label = if self.sstv_show_prior_received {
                "Hide prior sessions"
            } else {
                "Show prior sessions"
            };
            if ui.small_button(label).clicked() {
                self.sstv_show_prior_received = !self.sstv_show_prior_received;
                if self.sstv_show_prior_received {
                    self.load_prior_received_sstv_images();
                }
            }
            if ui.small_button("Prev").clicked() && count > 1 {
                let current = self
                    .sstv_active_received_id
                    .as_deref()
                    .and_then(|id| {
                        snapshot
                            .sstv_received_images
                            .iter()
                            .position(|image| image.id == id)
                    })
                    .unwrap_or(0);
                let next = if current + 1 >= count { 0 } else { current + 1 };
                self.sstv_active_received_id = snapshot
                    .sstv_received_images
                    .get(next)
                    .map(|image| image.id.clone());
            }
            if ui.small_button("Next").clicked() && count > 1 {
                let current = self
                    .sstv_active_received_id
                    .as_deref()
                    .and_then(|id| {
                        snapshot
                            .sstv_received_images
                            .iter()
                            .position(|image| image.id == id)
                    })
                    .unwrap_or(0);
                let next = if current == 0 { count - 1 } else { current - 1 };
                self.sstv_active_received_id = snapshot
                    .sstv_received_images
                    .get(next)
                    .map(|image| image.id.clone());
            }
            if ui.small_button("Remove").clicked() {
                if let Some(active_id) = self.sstv_active_received_id.clone() {
                    let mut shared = self.state.lock().expect("ui state lock poisoned");
                    shared
                        .sstv_received_images
                        .retain(|image| image.id != active_id);
                    shared.sstv_received_revision = shared.sstv_received_revision.wrapping_add(1);
                    self.sstv_active_received_id = shared
                        .sstv_received_images
                        .front()
                        .map(|image| image.id.clone());
                }
            }
        });
        if snapshot.sstv_received_images.is_empty() {
            ui.label(
                RichText::new("No received SSTV frames yet")
                    .small()
                    .color(theme_muted(ui)),
            );
            return;
        }
        egui::ScrollArea::horizontal()
            .id_salt("sstv_received_carousel")
            .max_height(80.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for image in &snapshot.sstv_received_images {
                        let selected = self
                            .sstv_active_received_id
                            .as_deref()
                            .map(|id| id == image.id)
                            .unwrap_or_else(|| {
                                snapshot
                                    .sstv_received_images
                                    .front()
                                    .is_some_and(|front| front.id == image.id)
                            });
                        let response =
                            if let Some(texture) = self.sstv_received_textures.get(&image.id) {
                                let aspect = image.height as f32 / image.width.max(1) as f32;
                                ui.add(
                                    egui::Button::image(egui::Image::new((
                                        texture.id(),
                                        egui::vec2(72.0, 72.0 * aspect),
                                    )))
                                    .selected(selected),
                                )
                            } else {
                                ui.selectable_label(
                                    selected,
                                    image.mode.map(|mode| mode.name()).unwrap_or("RX"),
                                )
                            };
                        if response.clicked() {
                            self.sstv_active_received_id = Some(image.id.clone());
                        }
                    }
                });
            });
        if self.sstv_active_received_id.is_none() {
            self.sstv_active_received_id = snapshot
                .sstv_received_images
                .front()
                .map(|image| image.id.clone());
        }
    }

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
        self.update_received_sstv_textures(ctx, &snapshot);
        if snapshot.sstv_revision != self.sstv_texture_revision
            && snapshot.sstv_width > 0
            && snapshot.sstv_height > 0
            && snapshot.sstv_rgb.len() == snapshot.sstv_width * snapshot.sstv_height * 3
        {
            let image = ColorImage::from_rgb(
                [snapshot.sstv_width, snapshot.sstv_height],
                &snapshot.sstv_rgb,
            );
            self.sstv_texture = Some(ctx.load_texture("sstv-frame", image, TextureOptions::LINEAR));
            self.sstv_texture_revision = snapshot.sstv_revision;
        }
        if self.sstv_tx_revision != self.sstv_tx_texture_revision
            && self.sstv_tx_width > 0
            && self.sstv_tx_height > 0
            && self.sstv_tx_rgb.len() == self.sstv_tx_width * self.sstv_tx_height * 3
        {
            let image =
                ColorImage::from_rgb([self.sstv_tx_width, self.sstv_tx_height], &self.sstv_tx_rgb);
            self.sstv_tx_texture =
                Some(ctx.load_texture("sstv-tx-frame", image, TextureOptions::LINEAR));
            self.sstv_tx_texture_revision = self.sstv_tx_revision;
        }

        let mut rx_mode = snapshot.sstv_rx_mode;
        ui.horizontal_wrapped(|ui| {
            ui.heading("📺 SSTV");
            ui.small_button("?").on_hover_text(
                "Calling frequencies are voluntary band-plan centers; confirm your license privileges and a clear channel before transmitting.\n\nAuto Target searches the filter for a complete shifted VIS header; Auto (VIS) then selects the image mode. Start before the header. Clicking the signal's 1900 Hz leader/pixel center switches to manual targeting.",
            );
            let debug_active = snapshot.sstv_debug_capture_requested;
            let debug_label = if debug_active {
                "🐞 DEBUG ARMED"
            } else {
                "🐞 DEBUG RX"
            };
            if ui
                .small_button(debug_label)
                .on_hover_text(
                    "Capture audio from the next SSTV trigger through image completion and save it with reception metadata",
                )
                .clicked()
            {
                let mut shared = self.state.lock().expect("ui state lock poisoned");
                shared.sstv_debug_capture_requested = !shared.sstv_debug_capture_requested;
                shared.sstv_debug_status = if shared.sstv_debug_capture_requested {
                    "Debug capture requested for next SSTV reception".to_string()
                } else {
                    "Debug capture cancelled".to_string()
                };
            }
            ui.separator();
            let old_auto_target = self.sstv_auto_target;
            ui.checkbox(&mut self.sstv_auto_target, "AUTO TARGET");
            if self.sstv_auto_target != old_auto_target {
                let mut shared = self.state.lock().expect("ui state lock poisoned");
                shared.sstv_auto_target = self.sstv_auto_target;
                shared.sstv_locked_offset_hz = None;
                shared.sstv_progress = None;
                tracing::info!(
                    auto_target = self.sstv_auto_target,
                    manual_offset_hz = self.sstv_tuning_offset_hz,
                    "SSTV receive targeting changed"
                );
            }
            ui.label(
                RichText::new(if self.sstv_auto_target {
                    snapshot
                        .sstv_locked_offset_hz
                        .map(|offset| format!("LOCK {offset:+} Hz"))
                        .unwrap_or_else(|| "SCANNING BASEBAND".to_string())
                } else {
                    format!("MANUAL {:+} Hz", self.sstv_tuning_offset_hz)
                })
                .small()
                .color(if snapshot.sstv_locked_offset_hz.is_some() {
                    theme_success(ui)
                } else {
                    theme_accent(ui)
                }),
            );
            if ui.small_button("VIEW SSTV LOG").clicked() {
                self.app_log_filter = "SSTV".to_string();
                self.app_log_level_filter = AppLogLevelFilter::All;
                self.signal_panel_tab = SignalPanelTab::AppLog;
            }
            if ui
                .button(RichText::new("⟳ DROP + REACQUIRE").color(theme_warning(ui)))
                .clicked()
            {
                self.sstv_auto_target = true;
                let mut shared = self.state.lock().expect("ui state lock poisoned");
                shared.sstv_auto_target = true;
                shared.sstv_locked_offset_hz = None;
                shared.sstv_detected_mode = None;
                shared.sstv_progress = None;
                shared.sstv_reset_generation = shared.sstv_reset_generation.wrapping_add(1);
                shared.sstv_status = "DROPPED: reacquiring across the audio filter".to_string();
                tracing::info!("SSTV receive dropped by operator; reacquiring");
            }
            ui.separator();
            ui.label(RichText::new("RX mode").strong());
            egui::ComboBox::from_id_salt("sstv_rx_mode")
                .selected_text(rx_mode.map(|mode| mode.name()).unwrap_or("Auto (VIS)"))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut rx_mode, None, "Auto (VIS)");
                    for &mode in qsonaut_sstv::supported_modes() {
                        ui.selectable_value(&mut rx_mode, Some(mode), mode.name());
                    }
                });
            ui.separator();
            ui.label(format!(
                "Detected: {}",
                snapshot
                    .sstv_detected_mode
                    .map(|mode| mode.name())
                    .unwrap_or("waiting for header")
            ));
        });
        if rx_mode != snapshot.sstv_rx_mode {
            let mut shared = self.state.lock().expect("ui state lock poisoned");
            shared.sstv_rx_mode = rx_mode;
            shared.sstv_detected_mode = None;
            shared.sstv_progress = None;
            shared.sstv_status = format!(
                "RX MODE: {} · waiting for a complete VIS header",
                rx_mode.map(|mode| mode.name()).unwrap_or("Auto (VIS)")
            );
            tracing::info!(
                receive_mode = rx_mode.map(|mode| mode.name()).unwrap_or("Auto (VIS)"),
                "SSTV receive mode changed"
            );
        }
        ui.add_space(4.0);

        // Reserve the safety strip and make the two work areas own only the
        // remaining viewport. Each column then manages its own overflow rather
        // than making the entire SSTV workspace taller than the central panel.
        let body_height = fitted_sstv_body_height(ui.available_height());
        ui.horizontal(|ui| {
            let gap = 12.0;
            let deck_width = (ui.available_width() - gap).max(2.0);
            let left_width = deck_width * self.sstv_rx_width_percent as f32 / 100.0;
            let right_width = (deck_width - left_width).max(1.0);
            let (left_rect, _) =
                ui.allocate_exact_size(egui::vec2(left_width, body_height), egui::Sense::hover());
            let (divider_rect, divider_response) =
                ui.allocate_exact_size(egui::vec2(gap, body_height), egui::Sense::drag());
            let divider_response =
                divider_response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
            if divider_response.dragged() {
                let delta_percent = ctx.input(|input| input.pointer.delta().x) / deck_width * 100.0;
                self.sstv_rx_width_percent = ((self.sstv_rx_width_percent as f32 + delta_percent)
                    .round() as i32)
                    .clamp(36, 72) as u8;
                ctx.request_repaint();
            }
            ui.painter().rect_filled(
                divider_rect.shrink2(egui::vec2(4.0, 4.0)),
                2.0,
                if divider_response.hovered() || divider_response.dragged() {
                    theme_accent(ui)
                } else {
                    ui.visuals().widgets.inactive.bg_stroke.color
                },
            );
            let (right_rect, _) =
                ui.allocate_exact_size(egui::vec2(right_width, body_height), egui::Sense::hover());
            let mut left = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("sstv_rx_deck")
                    .max_rect(left_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            let mut right = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("sstv_tx_deck")
                    .max_rect(right_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );

            egui::Frame::group(left.style()).show(&mut left, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(RichText::new("📡 LIVE RX").strong().color(theme_accent(ui)));
                ui.label(RichText::new(&snapshot.sstv_status).monospace());
                if let Some(progress) = snapshot.sstv_progress {
                    ui.add(
                        egui::ProgressBar::new(progress)
                            .desired_width(ui.available_width())
                            .text(format!("Receiving {:.0}%", progress * 100.0)),
                    );
                }
                if let Some(path) = &snapshot.sstv_saved_path {
                    ui.label(
                        RichText::new(format!("Saved: {path}"))
                            .small()
                            .color(theme_muted(ui)),
                    );
                }
                ui.add_space(3.0);
                let carousel_height = if snapshot.sstv_received_images.is_empty() {
                    58.0
                } else {
                    116.0
                };
                let preview_height = (ui.available_height() - carousel_height).max(96.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), preview_height),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        egui::Frame::dark_canvas(ui.style()).show(ui, |ui| {
                            let available_width = ui.available_width().max(1.0);
                            let available_height = ui.available_height().max(1.0);
                            let aspect = snapshot.sstv_height as f32
                                / snapshot.sstv_width.max(1) as f32;
                            let image_width =
                                available_width.min(available_height / aspect.max(0.01));
                            let size = egui::vec2(image_width, image_width * aspect);
                            if let Some(texture) = &self.sstv_texture {
                                ui.add(egui::Image::new((texture.id(), size)).corner_radius(5.0));
                            } else {
                                ui.allocate_ui_with_layout(
                                    size,
                                    egui::Layout::centered_and_justified(
                                        egui::Direction::TopDown,
                                    ),
                                    |ui| {
                                        ui.label(
                                            RichText::new("Listening for an SSTV frame")
                                                .color(theme_muted(ui)),
                                        )
                                    },
                                );
                            }
                        });
                    },
                );
                self.draw_received_sstv_carousel(ui, &snapshot);
            });

            egui::ScrollArea::vertical()
                .id_salt("sstv_local_image_lab_scroll")
                .max_height(body_height)
                .auto_shrink([false, false])
                .show(&mut right, |ui| {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(
                            RichText::new("📤 TX IMAGE")
                                .strong()
                                .color(theme_accent(ui)),
                        );
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("📂 OPEN IMAGE…").clicked() {
                                self.sstv_file_dialog.pick_file();
                            }
                            ui.add(
                                egui::TextEdit::singleline(&mut self.sstv_image_path)
                                    .hint_text("PNG/JPEG path")
                                    .desired_width((ui.available_width() - 62.0).max(120.0)),
                            );
                            if ui.small_button("LOAD").clicked() {
                                match std::fs::read(self.sstv_image_path.trim()) {
                                    Ok(bytes) => self.install_sstv_image(&bytes, "Loaded image"),
                                    Err(error) => {
                                        self.local_image_status =
                                            format!("Image load failed: {error}")
                                    }
                                }
                            }
                        });
                        if let Some(texture) = &self.sstv_tx_texture {
                            let aspect =
                                self.sstv_tx_height as f32 / self.sstv_tx_width.max(1) as f32;
                            let width = ui.available_width().min(230.0 / aspect.max(0.01));
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(width, width * aspect),
                                egui::Sense::drag(),
                            );
                            ui.painter().image(
                                texture.id(),
                                rect,
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                Color32::WHITE,
                            );
                            if response.dragged() {
                                let delta = response.drag_delta();
                                self.sstv_background_pan_x += delta.x * self.sstv_tx_width as f32
                                    / rect.width().max(1.0);
                                self.sstv_background_pan_y += delta.y * self.sstv_tx_height as f32
                                    / rect.height().max(1.0);
                                self.rebuild_sstv_tx_image();
                            }
                            ui.label(
                                RichText::new("Drag image to reposition")
                                    .small()
                                    .color(theme_muted(ui)),
                            );
                        } else {
                            ui.label(
                                RichText::new("Open or generate an image to prepare TX")
                                    .small()
                                    .color(theme_muted(ui)),
                            );
                        }
                        ui.label(RichText::new("Overlay layers").strong());
                        ui.horizontal_wrapped(|ui| {
                            let mut changed = false;
                            changed |= ui
                                .checkbox(&mut self.sstv_overlay_callsign, "Callsign")
                                .changed();
                            changed |= ui.checkbox(&mut self.sstv_overlay_grid, "Grid").changed();
                            changed |= ui
                                .checkbox(&mut self.sstv_overlay_frequency, "Frequency")
                                .changed();
                            changed |= ui
                                .checkbox(&mut self.sstv_overlay_mode, "SSTV mode")
                                .changed();
                            if ui.small_button("Reset layers").clicked() {
                                self.reset_sstv_overlay();
                            } else if changed {
                                self.rebuild_sstv_tx_image();
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Background");
                            if ui
                                .add(
                                    egui::Slider::new(
                                        &mut self.sstv_background_zoom,
                                        0.25..=3.0,
                                    )
                                    .text("Zoom"),
                                )
                                .changed()
                            {
                                self.rebuild_sstv_tx_image();
                            }
                            if ui.small_button("Reset view").clicked() {
                                self.sstv_background_zoom = 1.0;
                                self.sstv_background_pan_x = 0.0;
                                self.sstv_background_pan_y = 0.0;
                                self.rebuild_sstv_tx_image();
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Overlay background");
                            if ui
                                .color_edit_button_srgba(&mut self.sstv_overlay_background)
                                .changed()
                            {
                                self.rebuild_sstv_tx_image();
                            }
                            if ui
                                .add(
                                    egui::Slider::new(
                                        &mut self.sstv_overlay_background_opacity,
                                        0.0..=1.0,
                                    )
                                    .text("Opacity"),
                                )
                                .changed()
                            {
                                self.rebuild_sstv_tx_image();
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Overlay corner");
                            let previous_corner = self.sstv_overlay_corner;
                            egui::ComboBox::from_id_salt("sstv-overlay-corner")
                                .selected_text(self.sstv_overlay_corner.label())
                                .show_ui(ui, |ui| {
                                    for corner in SstvOverlayCorner::ALL {
                                        ui.selectable_value(
                                            &mut self.sstv_overlay_corner,
                                            corner,
                                            corner.label(),
                                        );
                                    }
                                });
                            if self.sstv_overlay_corner != previous_corner {
                                self.rebuild_sstv_tx_image();
                            }
                        });
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new("TX mode").strong());
                            let previous_mode = self.sstv_tx_mode;
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
                            if self.sstv_tx_mode != previous_mode {
                                self.rebuild_sstv_tx_image();
                            }
                        });
                        ui.separator();
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new("🧠 AI IMAGE").strong());
                            ui.label(format!(
                                "{} · {}",
                                self.local_image_settings.provider.label(),
                                if self.local_image_settings.image_model.is_empty() {
                                    "no model selected"
                                } else {
                                    &self.local_image_settings.image_model
                                }
                            ));
                            if ui.small_button("⚙ CONFIGURE AI").clicked() {
                                self.signal_panel_tab = SignalPanelTab::Ai;
                                self.show_signal_panel = true;
                            }
                        });
                        if self.sstv_ai_prompt.is_empty() {
                            self.sstv_ai_prompt = self.sstv_activity_prompt(&snapshot);
                        }
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Pipeline");
                            egui::ComboBox::from_id_salt("sstv-ai-pipeline-mode")
                                .selected_text(self.sstv_ai_pipeline_mode.label())
                                .show_ui(ui, |ui| {
                                    for mode in SstvAiPipelineMode::ALL {
                                        ui.selectable_value(
                                            &mut self.sstv_ai_pipeline_mode,
                                            mode,
                                            mode.label(),
                                        );
                                    }
                                });
                        });
                        if self.sstv_ai_pipeline_mode == SstvAiPipelineMode::StationQsl {
                            ui.add(
                                egui::TextEdit::multiline(&mut self.sstv_ai_prompt)
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(5)
                                    .hint_text("Describe the image to transmit"),
                            );
                            ui.horizontal(|ui| {
                                if ui.button("Use current activity").clicked() {
                                    self.sstv_ai_prompt = self.sstv_activity_prompt(&snapshot);
                                }
                                if ui
                                    .add_enabled(
                                        !self.local_image_settings.image_model.is_empty(),
                                        egui::Button::new("✨ GENERATE LOCALLY"),
                                    )
                                    .clicked()
                                {
                                    self.generate_local_sstv_image();
                                }
                            });
                        }
                        if self.sstv_ai_pipeline_mode != SstvAiPipelineMode::StationQsl {
                            if let Some(image) = self.selected_received_sstv_image(&snapshot) {
                                ui.label(
                                    RichText::new(format!(
                                        "Selected RX: {} ; {}x{} ; {} ; rx {} ; {}",
                                        image.mode.map(|mode| mode.name()).unwrap_or("SSTV"),
                                        image.width,
                                        image.height,
                                        image
                                            .frequency_hz
                                            .map(|hz| format!("{:.3} MHz", hz as f64 / 1_000_000.0))
                                            .unwrap_or_else(|| "RF unknown".to_string()),
                                        image.received_unix_ms,
                                        image
                                            .path
                                            .as_deref()
                                            .unwrap_or("not saved"),
                                    ))
                                    .small(),
                                );
                                if let Some(analysis) = &image.analysis {
                                    ui.label(RichText::new("Analysis").strong());
                                    ui.label(analysis);
                                }
                            } else {
                                ui.label(
                                    RichText::new("Select a received SSTV image first")
                                        .small()
                                        .color(theme_warning(ui)),
                                );
                            }
                            if self.sstv_ai_pipeline_mode
                                == SstvAiPipelineMode::AnalyzeReceived
                                && ui
                                    .add_enabled(
                                        self.selected_received_sstv_image(&snapshot).is_some()
                                            && !self
                                                .local_image_settings
                                                .vision_model
                                                .is_empty(),
                                        egui::Button::new("Analyze selected RX"),
                                    )
                                    .clicked()
                            {
                                self.analyze_selected_received_sstv(&snapshot);
                            }
                            if self.sstv_ai_pipeline_mode
                                == SstvAiPipelineMode::ReinterpretReceived
                            {
                                if self.sstv_reinterpret_prompt.is_empty() {
                                    self.sstv_reinterpret_prompt =
                                        self.sstv_reinterpretation_prompt();
                                }
                                ui.add(
                                    egui::TextEdit::multiline(
                                        &mut self.sstv_reinterpret_prompt,
                                    )
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(4)
                                    .hint_text("Describe the reinterpretation"),
                                );
                                ui.label(
                                    RichText::new("Image-generation models often render text inaccurately; verify the TX preview before arming.")
                                        .small()
                                        .color(theme_warning(ui)),
                                );
                                if ui
                                    .add_enabled(
                                        self.selected_received_sstv_image(&snapshot).is_some()
                                            && !self
                                                .local_image_settings
                                                .vision_model
                                                .is_empty()
                                            && !self.local_image_settings.edit_model.is_empty(),
                                        egui::Button::new("Reinterpret selected RX"),
                                    )
                                    .clicked()
                                {
                                    self.reinterpret_selected_received_sstv(&snapshot);
                                }
                            }
                        }
                        ui.label(
                            RichText::new(&self.local_image_status)
                                .small()
                                .color(theme_muted(ui)),
                        );
                    })
                });
        });

        ui.add_space(5.0);
        let has_frame = self.sstv_tx_width > 0
            && self.sstv_tx_height > 0
            && self.sstv_tx_rgb.len() == self.sstv_tx_width * self.sstv_tx_height * 3;
        egui::Frame::group(ui.style())
            .fill(if self.sstv_tx_armed {
                Color32::from_rgb(76, 31, 25)
            } else {
                Color32::from_rgb(20, 43, 52)
            })
            .stroke(egui::Stroke::new(
                2.0_f32,
                if self.sstv_tx_armed {
                    Color32::LIGHT_RED
                } else {
                    Color32::LIGHT_BLUE
                },
            ))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
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
                        let rgb = self.sstv_tx_rgb.clone();
                        self.start_sstv_tx(
                            self.sstv_tx_mode,
                            self.sstv_tx_width,
                            self.sstv_tx_height,
                            &rgb,
                        );
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
            "Create bold, high-contrast amateur radio SSTV QSL artwork for callsign {} in {} {}. Current activity: {:.3} MHz SSTV {}. Rig: {}. Antenna: {}. Station notes: {}. General prompt context: {}. SSTV image requirements: {}. Model notes: {}. Use a striking radio-space aesthetic, one strong central subject, large readable callsign, no tiny text, and a composition that survives analog SSTV transmission.",
            self.station_callsign_or_default(),
            self.station_qth.trim(),
            self.station_grid_or_default(),
            snapshot.frequency_hz.unwrap_or(14_230_000) as f64 / 1_000_000.0,
            self.sstv_tx_mode.name(),
            self.station_rig.trim(),
            self.station_antenna.trim(),
            self.station_notes.trim(),
            self.llm_prompt_context.trim(),
            self.sstv_image_requirements.trim(),
            self.llm_model_notes.trim(),
        )
    }

    fn sstv_vision_instruction(&self) -> String {
        "Inspect this received amateur-radio SSTV image. Describe the main subject, dominant colors, visible station or location text if present, composition, and any artifacts relevant to creating simple high-contrast QSL artwork. Do not invent callsigns or labels.".to_string()
    }

    fn sstv_reinterpretation_prompt(&self) -> String {
        format!(
            "Reinterpret this received amateur-radio SSTV image as clean, high-contrast QSL artwork. Preserve the main subject and dominant colors while simplifying the composition for analog SSTV transmission. Use the provided station callsign and grid only when explicitly supplied. Do not invent callsigns, labels, or text. Avoid tiny text and fine detail.\n\nCallsign: {}\nGrid: {}\nStation notes: {}\nModel notes: {}",
            self.station_callsign.trim(),
            self.station_grid.trim(),
            self.station_notes.trim(),
            self.llm_model_notes.trim(),
        )
    }

    pub(crate) fn refresh_local_image_models(&mut self) {
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
        let model = match self.selected_model_for_role(LocalModelRole::Image) {
            Ok(model) => model,
            Err(error) => {
                self.local_image_status = error.to_string();
                return;
            }
        };
        self.local_image_settings.model = self.local_image_settings.image_model.clone();
        let _ = self.local_image_settings.save();
        let settings = self.local_image_settings.clone();
        let prompt = self.sstv_ai_prompt.clone();
        let sender = self.local_image_event_tx.clone();
        self.local_image_status = format!(
            "{} is generating with {}… this can take several minutes",
            settings.provider.label(),
            model
        );
        thread::spawn(move || {
            let result = local_ai::generate_with_model(&settings, &model, &prompt)
                .map_err(|error| error.to_string());
            let _ = sender.send(LocalImageEvent::Generated(result));
        });
    }

    fn analyze_selected_received_sstv(&mut self, snapshot: &GuiState) {
        if let Err(error) =
            local_ai::validate_loopback_endpoint(self.local_image_settings.endpoint())
        {
            self.local_image_status = error.to_string();
            return;
        }
        let model = match self.selected_model_for_role(LocalModelRole::Vision) {
            Ok(model) => model,
            Err(error) => {
                self.local_image_status = error.to_string();
                return;
            }
        };
        let (image_id, image_png) = match self.selected_received_sstv_png(snapshot) {
            Ok(image) => image,
            Err(error) => {
                self.local_image_status = error.to_string();
                return;
            }
        };
        let settings = self.local_image_settings.clone();
        let instruction = self.sstv_vision_instruction();
        let sender = self.local_image_event_tx.clone();
        self.local_image_status = format!(
            "{} is analyzing selected RX with {model}",
            settings.provider.label()
        );
        thread::spawn(move || {
            let result = local_ai::analyze_image(&settings, &model, &image_png, &instruction)
                .map(|analysis| (image_id, analysis))
                .map_err(|error| error.to_string());
            let _ = sender.send(LocalImageEvent::Vision(result));
        });
    }

    fn reinterpret_selected_received_sstv(&mut self, snapshot: &GuiState) {
        if let Err(error) =
            local_ai::validate_loopback_endpoint(self.local_image_settings.endpoint())
        {
            self.local_image_status = error.to_string();
            return;
        }
        let vision_model = match self.selected_model_for_role(LocalModelRole::Vision) {
            Ok(model) => model,
            Err(error) => {
                self.local_image_status = error.to_string();
                return;
            }
        };
        let edit_model = match self.selected_model_for_role(LocalModelRole::Edit) {
            Ok(model) => model,
            Err(error) => {
                self.local_image_status = error.to_string();
                return;
            }
        };
        let (image_id, image_png) = match self.selected_received_sstv_png(snapshot) {
            Ok(image) => image,
            Err(error) => {
                self.local_image_status = error.to_string();
                return;
            }
        };
        let settings = self.local_image_settings.clone();
        let instruction = self.sstv_vision_instruction();
        let prompt_seed = self.sstv_reinterpret_prompt.clone();
        let sender = self.local_image_event_tx.clone();
        self.local_image_status = format!(
            "{} is analyzing and reinterpreting selected RX; this can take several minutes",
            settings.provider.label()
        );
        thread::spawn(move || {
            let analysis =
                match local_ai::analyze_image(&settings, &vision_model, &image_png, &instruction) {
                    Ok(analysis) => analysis,
                    Err(error) => {
                        let _ = sender.send(LocalImageEvent::Vision(Err(error.to_string())));
                        return;
                    }
                };
            let _ = sender.send(LocalImageEvent::Vision(Ok((image_id, analysis.clone()))));
            let prompt = format!("{prompt_seed}\n\nVision-model analysis:\n{analysis}");
            let result = local_ai::edit_image(&settings, &edit_model, &prompt, image_png)
                .map_err(|error| error.to_string());
            let _ = sender.send(LocalImageEvent::Edited(result));
        });
    }

    pub(crate) fn poll_local_image_events(&mut self) {
        while let Ok(event) = self.local_image_event_rx.try_recv() {
            match event {
                LocalImageEvent::Models(Ok(models)) => {
                    self.local_image_models = models;
                    for role in [
                        LocalModelRole::Vision,
                        LocalModelRole::Image,
                        LocalModelRole::Edit,
                    ] {
                        let selected = self.selected_model_id_for_role(role).to_string();
                        if !selected.trim().is_empty()
                            && local_ai::model_for_role(
                                &self.local_image_models,
                                &selected,
                                role,
                            )
                            .is_err()
                        {
                            match role {
                                LocalModelRole::Vision => {
                                    self.local_image_settings.vision_model.clear()
                                }
                                LocalModelRole::Image => {
                                    self.local_image_settings.image_model.clear()
                                }
                                LocalModelRole::Edit => {
                                    self.local_image_settings.edit_model.clear()
                                }
                            }
                            tracing::warn!(
                                role = %role.label(),
                                previous_model = %selected,
                                "cleared unavailable local AI model selection"
                            );
                        }
                    }
                    let vision_count = self
                        .local_image_models
                        .iter()
                        .filter(|model| model.supports(LocalModelRole::Vision))
                        .count();
                    let image_count = self
                        .local_image_models
                        .iter()
                        .filter(|model| model.supports(LocalModelRole::Image))
                        .count();
                    let edit_count = self
                        .local_image_models
                        .iter()
                        .filter(|model| model.supports(LocalModelRole::Edit))
                        .count();
                    self.local_image_status = format!(
                        "Found {} local model{}; compatible roles: vision {}, image {}, edit {}",
                        self.local_image_models.len(),
                        if self.local_image_models.len() == 1 {
                            ""
                        } else {
                            "s"
                        },
                        vision_count,
                        image_count,
                        edit_count,
                    );
                    tracing::info!(
                        provider = %self.local_image_settings.provider.label(),
                        model_count = self.local_image_models.len(),
                        "local AI model discovery completed"
                    );
                    if vision_count == 0 {
                        tracing::warn!(
                            provider = %self.local_image_settings.provider.label(),
                            "no compatible vision/context models were advertised"
                        );
                    }
                    if image_count == 0 {
                        tracing::warn!(
                            provider = %self.local_image_settings.provider.label(),
                            "no compatible image-generation models were advertised"
                        );
                    }
                    if edit_count == 0 {
                        tracing::warn!(
                            provider = %self.local_image_settings.provider.label(),
                            "no compatible image-editing models were advertised"
                        );
                    }
                    let _ = self.local_image_settings.save();
                }
                LocalImageEvent::Models(Err(error)) => {
                    tracing::warn!(error = %error, "local AI model discovery failed");
                    self.local_image_status = format!("Model discovery failed: {error}");
                }
                LocalImageEvent::Vision(Err(error)) => {
                    tracing::warn!(error = %error, "local AI vision analysis failed");
                    self.local_image_status = format!("Vision analysis failed: {error}");
                }
                LocalImageEvent::Generated(Err(error)) => {
                    tracing::warn!(error = %error, "local AI image generation failed");
                    self.local_image_status = format!("Image generation failed: {error}");
                }
                LocalImageEvent::Edited(Err(error)) => {
                    tracing::warn!(error = %error, "local AI image reinterpretation failed");
                    self.local_image_status = format!("Image reinterpretation failed: {error}");
                }
                LocalImageEvent::Vision(Ok((image_id, analysis))) => {
                    let mut shared = self.state.lock().expect("ui state lock poisoned");
                    if let Some(image) = shared
                        .sstv_received_images
                        .iter_mut()
                        .find(|image| image.id == image_id)
                    {
                        image.analysis = Some(analysis);
                        shared.sstv_received_revision =
                            shared.sstv_received_revision.wrapping_add(1);
                        self.local_image_status =
                            "Vision analysis saved for selected received image".to_string();
                    } else {
                        self.local_image_status =
                            "Vision analysis finished, but the received image was removed"
                                .to_string();
                    }
                }
                LocalImageEvent::Generated(Ok(bytes)) => {
                    self.install_sstv_image(&bytes, "Generated locally");
                }
                LocalImageEvent::Edited(Ok(bytes)) => {
                    self.install_sstv_image(&bytes, "Reinterpreted locally");
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
                self.sstv_tx_rgb = rgb.into_raw();
                self.sstv_tx_base_rgb = self.sstv_tx_rgb.clone();
                self.sstv_tx_width = qsonaut_sstv::WIDTH;
                self.sstv_tx_height = qsonaut_sstv::HEIGHT;
                self.sstv_tx_revision = self.sstv_tx_revision.wrapping_add(1);
                self.rebuild_sstv_tx_image();
                self.local_image_status = match saved {
                    Ok(()) => format!("{source}; 320×256 SSTV frame saved locally"),
                    Err(error) => format!("{source}; local save failed: {error}"),
                };
            }
            Err(error) => self.local_image_status = format!("Image decode failed: {error}"),
        }
    }

    fn start_sstv_tx(
        &mut self,
        mode: qsonaut_sstv::SstvMode,
        width: usize,
        height: usize,
        rgb: &[u8],
    ) {
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
        match qsonaut_sstv::encode_rgb_mode_12k(mode, width as u32, height as u32, rgb) {
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
