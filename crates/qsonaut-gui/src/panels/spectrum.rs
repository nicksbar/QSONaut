use super::super::*;

impl QsonautGuiApp {
    pub(in super::super) fn draw_radio_waterfall(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        snapshot: &GuiState,
    ) {
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(
                &mut self.radio_scope_view,
                RadioScopeView::Narrow,
                "Narrow passband",
            );
            ui.selectable_value(
                &mut self.radio_scope_view,
                RadioScopeView::Overview,
                "Active band overview",
            );
            ui.add(
                egui::Slider::new(&mut self.radio_scope_contrast, 0.7..=3.0)
                    .text("intensity")
                    .clamping(egui::SliderClamping::Always),
            );
            let previous_theme = self.waterfall_theme;
            egui::ComboBox::from_id_salt("waterfall_theme")
                .selected_text(self.waterfall_theme.label())
                .show_ui(ui, |ui| {
                    for theme in [
                        WaterfallTheme::RadioBlue,
                        WaterfallTheme::Inferno,
                        WaterfallTheme::Phosphor,
                        WaterfallTheme::Monochrome,
                    ] {
                        ui.selectable_value(&mut self.waterfall_theme, theme, theme.label());
                    }
                });
            if self.waterfall_theme != previous_theme {
                self.profile_dirty = true;
                self.persist_profile("Auto-saved");
            }
            ui.separator();
            ui.label(RichText::new("Sweep").strong());
            let mut tuning = self.display_tuning.lock().expect("tuning lock poisoned");
            if ui.selectable_label(tuning.auto_visual, "Auto").clicked() {
                tuning.auto_visual = true;
            }
            for (speed, label) in [
                (WaterfallSpeed::Fast, "Fast"),
                (WaterfallSpeed::Mid, "Mid"),
                (WaterfallSpeed::Slow, "Slow"),
            ] {
                if ui
                    .selectable_label(
                        !tuning.auto_visual && tuning.waterfall_speed == speed,
                        label,
                    )
                    .clicked()
                {
                    tuning.auto_visual = false;
                    tuning.waterfall_speed = speed;
                }
            }
        });
        egui::CollapsingHeader::new("Advanced radio scope")
            .default_open(false)
            .show(ui, |ui| {
                egui::Frame::group(ui.style())
                    .fill(ui.visuals().faint_bg_color)
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.checkbox(&mut self.radio_scope_hold, "Hold")
                                .on_hover_text("Freeze or resume the IC-7300 scope");
                            ui.add(
                                egui::Slider::new(
                                    &mut self.radio_scope_reference_tenths_db,
                                    -200..=200,
                                )
                                .step_by(5.0)
                                .custom_formatter(|value, _| format!("{:.1} dB", value / 10.0))
                                .text("reference"),
                            )
                            .on_hover_text(
                                "Shift the IC-7300 scope reference level from -20 to +20 dB",
                            );
                        });
                        ui.small("These controls are sent directly to the radio.");
                    });
            });

        let (rows, source_revision, source_bins, render_bins) = if self.radio_scope_view
            == RadioScopeView::Narrow
        {
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(
                    &mut self.radio_scope_lock_if_to_filter,
                    "Match center span to selected FIL",
                );
                ui.checkbox(&mut self.radio_scope_vbw_wide, "VBW wide")
                    .on_hover_text(
                        "Video bandwidth: wide responds faster; narrow smooths noise but reacts more slowly",
                    );
                if self.radio_scope_lock_if_to_filter {
                    let auto_span = scope_span_for_filter(&snapshot.mode, snapshot.filter);
                    self.radio_scope_span_code = auto_span;
                    ui.label(
                        RichText::new(format!("auto span {}", scope_span_label(auto_span)))
                            .small()
                            .color(Color32::LIGHT_BLUE),
                    );
                } else {
                    egui::ComboBox::from_id_salt("radio_scope_span")
                        .selected_text(scope_span_label(self.radio_scope_span_code))
                        .show_ui(ui, |ui| {
                            for (code, label) in [
                                (0u8, "±2.5 kHz"),
                                (1u8, "±5 kHz"),
                                (2u8, "±10 kHz"),
                                (3u8, "±25 kHz"),
                                (4u8, "±50 kHz"),
                                (5u8, "±100 kHz"),
                                (6u8, "±250 kHz"),
                                (7u8, "±500 kHz"),
                            ] {
                                ui.selectable_value(&mut self.radio_scope_span_code, code, label);
                            }
                        });
                }
            });
            if !self.civ_spectrum_on {
                return;
            }

            let source_bins = snapshot
                .radio_waterfall_rows
                .back()
                .map(|row| row.len())
                .unwrap_or(RADIO_WF_WIDTH)
                .clamp(64, MAX_RADIO_WF_BINS);
            let render_bins = source_bins;
            (
                &snapshot.radio_waterfall_rows,
                snapshot.radio_waterfall_revision,
                source_bins,
                render_bins,
            )
        } else {
            if let Some((low, high, label)) = band_edges_for_frequency(snapshot.frequency_hz) {
                ui.label(
                    RichText::new(format!(
                        "Active band edges: {label} {:.3}–{:.3} MHz",
                        low as f64 / 1_000_000.0,
                        high as f64 / 1_000_000.0,
                    ))
                    .small()
                    .color(Color32::LIGHT_BLUE),
                );
            }
            let source_bins = snapshot
                .radio_waterfall_rows
                .back()
                .map(|row| row.len())
                .unwrap_or(RADIO_WF_WIDTH)
                .clamp(64, MAX_RADIO_WF_BINS);
            let render_bins = source_bins;
            (
                &snapshot.radio_waterfall_rows,
                snapshot.radio_waterfall_revision,
                source_bins,
                render_bins,
            )
        };

        let sideband_projection = if self.radio_scope_view == RadioScopeView::Narrow {
            scope_projection_for_mode(&snapshot.mode)
        } else {
            ScopeProjection::Full
        };
        // Keep the radio's native bins, but let the presentation fill
        // the horizontal monitor deck as the window is resized.
        let display_size = egui::vec2(
            ui.available_width().max(1.0),
            (ui.available_height() - 4.0).max(56.0),
        );

        if self.radio_waterfall_texture.is_none()
            || self.radio_waterfall_texture_revision != source_revision
            || self.radio_waterfall_texture_bins != render_bins
            || self.radio_waterfall_texture_view != self.radio_scope_view
            || self.radio_waterfall_texture_theme != self.waterfall_theme
        {
            let image = build_scope_waterfall_image(
                rows,
                render_bins,
                RADIO_WF_HEIGHT,
                self.waterfall_theme,
            );
            if let Some(tex) = &mut self.radio_waterfall_texture {
                tex.set(image, TextureOptions::LINEAR);
            } else {
                self.radio_waterfall_texture = Some(ctx.load_texture(
                    "qsonaut-radio-waterfall",
                    image,
                    TextureOptions::LINEAR,
                ));
            }
            self.radio_waterfall_texture_revision = source_revision;
            self.radio_waterfall_texture_bins = render_bins;
            self.radio_waterfall_texture_view = self.radio_scope_view;
            self.radio_waterfall_texture_theme = self.waterfall_theme;
        }

        if let Some(tex) = &self.radio_waterfall_texture {
            let response = ui.image((tex.id(), display_size));
            let dial_fraction = match self.radio_scope_view {
                RadioScopeView::Narrow => snapshot.frequency_hz.map(|frequency| {
                    let half_span = scope_span_hz(self.radio_scope_span_code);
                    let (low, high) = match sideband_projection {
                        ScopeProjection::Full => (
                            frequency.saturating_sub(half_span),
                            frequency.saturating_add(half_span),
                        ),
                        ScopeProjection::LowerSideband => {
                            (frequency.saturating_sub(half_span), frequency)
                        }
                        ScopeProjection::UpperSideband => {
                            (frequency, frequency.saturating_add(half_span))
                        }
                    };
                    // Calculate fraction based on frequency position within the displayed range
                    let range_width = high.saturating_sub(low);
                    if range_width > 0 {
                        let position_from_low = frequency.saturating_sub(low);
                        (position_from_low as f32 / range_width as f32).clamp(0.0, 1.0)
                    } else {
                        0.5 // Default to center if no range
                    }
                }),
                RadioScopeView::Overview => snapshot.frequency_hz.and_then(|frequency| {
                    band_edges_for_frequency(Some(frequency)).map(|(low, high, _)| {
                        ((frequency.saturating_sub(low)) as f32 / (high - low) as f32)
                            .clamp(0.0, 1.0)
                    })
                }),
            };
            if let Some(fraction) = dial_fraction {
                let dial_x = response.rect.left() + fraction * response.rect.width();
                ui.painter().line_segment(
                    [
                        egui::pos2(dial_x, response.rect.top()),
                        egui::pos2(dial_x, response.rect.bottom()),
                    ],
                    egui::Stroke::new(1.0_f32, Color32::from_rgb(245, 190, 70)),
                );
                ui.painter().text(
                    egui::pos2(dial_x + 4.0, response.rect.top() + 3.0),
                    egui::Align2::LEFT_TOP,
                    "VFO",
                    egui::TextStyle::Small.resolve(ui.style()),
                    Color32::from_rgb(245, 190, 70),
                );
            }
            let frequency_labels = match self.radio_scope_view {
                RadioScopeView::Narrow => snapshot.frequency_hz.map(|frequency| {
                    let half_span = scope_span_hz(self.radio_scope_span_code);
                    let (low, high) = match sideband_projection {
                        ScopeProjection::Full => (
                            frequency.saturating_sub(half_span),
                            frequency.saturating_add(half_span),
                        ),
                        ScopeProjection::LowerSideband => {
                            (frequency.saturating_sub(half_span), frequency)
                        }
                        ScopeProjection::UpperSideband => {
                            (frequency, frequency.saturating_add(half_span))
                        }
                    };
                    (
                        format!("{:.6}", low as f64 / 1e6),
                        format!("{:.6}", high as f64 / 1e6),
                    )
                }),
                RadioScopeView::Overview => {
                    band_edges_for_frequency(snapshot.frequency_hz).map(|(low, high, _)| {
                        (
                            format!("{:.3}", low as f64 / 1e6),
                            format!("{:.3} MHz", high as f64 / 1e6),
                        )
                    })
                }
            };
            if let Some((left, right)) = frequency_labels {
                let font = egui::TextStyle::Small.resolve(ui.style());
                ui.painter().text(
                    response.rect.left_bottom() + egui::vec2(4.0, -3.0),
                    egui::Align2::LEFT_BOTTOM,
                    left,
                    font.clone(),
                    Color32::WHITE,
                );
                ui.painter().text(
                    response.rect.right_bottom() + egui::vec2(-4.0, -3.0),
                    egui::Align2::RIGHT_BOTTOM,
                    right,
                    font,
                    Color32::WHITE,
                );
            }
        }

        let _ = (source_bins, render_bins);
    }

    pub(in super::super) fn draw_audio_waterfall(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        snapshot: &GuiState,
    ) {
        ui.heading("Audio Waterfall (RX Input / TX Output)");
        ui.separator();

        let bw_hz = filter_bandwidth_hz(&snapshot.mode, snapshot.filter);
        let display_bins = ((bw_hz.min(AUDIO_MAX_FREQ_HZ) as f32 / AUDIO_MAX_FREQ_HZ as f32)
            * AUDIO_BINS as f32)
            .round() as usize;
        let display_bins = display_bins.clamp(16, AUDIO_BINS);

        // Capture layout geometry before texture ops — available_width() can change mid-frame.
        let display_size = egui::vec2(
            ui.available_width().max(1.0),
            (ui.available_height() - 18.0).max(56.0),
        );

        if self.audio_waterfall_texture.is_none()
            || self.audio_waterfall_texture_revision != snapshot.audio_waterfall_revision
            || self.audio_waterfall_texture_bins != display_bins
            || self.audio_waterfall_texture_theme != self.waterfall_theme
        {
            let image = build_waterfall_image_with_theme(
                &snapshot.audio_waterfall_rows,
                display_bins,
                AUDIO_WF_HEIGHT,
                self.waterfall_theme,
            );
            if let Some(tex) = &mut self.audio_waterfall_texture {
                tex.set(image, TextureOptions::LINEAR);
            } else {
                self.audio_waterfall_texture = Some(ctx.load_texture(
                    "qsonaut-audio-waterfall",
                    image,
                    TextureOptions::LINEAR,
                ));
            }
            self.audio_waterfall_texture_revision = snapshot.audio_waterfall_revision;
            self.audio_waterfall_texture_bins = display_bins;
            self.audio_waterfall_texture_theme = self.waterfall_theme;
        }
        if let Some(tex) = &self.audio_waterfall_texture {
            let image_widget =
                egui::Image::new((tex.id(), display_size)).sense(egui::Sense::click());
            let response = ui.add(image_widget);

            if let Some(pos) = response.interact_pointer_pos() {
                let rel = ((pos.x - response.rect.left()) / response.rect.width()).clamp(0.0, 1.0);
                let capped_bw = bw_hz.clamp(100, AUDIO_MAX_FREQ_HZ);
                let pick_hz = ((rel * capped_bw as f32).round() as u32).clamp(100, capped_bw);

                if response.clicked() {
                    self.rx_tone_hz = pick_hz;
                    if !self.ft8_hold_tx_freq {
                        self.tx_tone_hz = pick_hz;
                    }
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                    self.profile_io_status = format!("RX audio cursor set: {} Hz", self.rx_tone_hz);
                }
                if response.secondary_clicked() {
                    self.tx_tone_hz = pick_hz;
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                    self.profile_io_status = format!("TX tone set: {} Hz", self.tx_tone_hz);
                }
            }

            let bw = bw_hz.clamp(1, AUDIO_MAX_FREQ_HZ) as f32;
            let rx_x = response.rect.left()
                + (self.rx_tone_hz.min(bw as u32) as f32 / bw) * response.rect.width();
            let tx_x = response.rect.left()
                + (self.tx_tone_hz.min(bw as u32) as f32 / bw) * response.rect.width();

            let channel_half_width = (12.5 / bw * response.rect.width()).max(2.0);
            let rx_band = egui::Rect::from_min_max(
                egui::pos2(rx_x - channel_half_width, response.rect.top()),
                egui::pos2(rx_x + channel_half_width, response.rect.bottom()),
            );
            ui.painter().rect_filled(
                rx_band,
                0.0,
                Color32::from_rgba_unmultiplied(80, 220, 110, 32),
            );
            if self.tx_tone_hz.abs_diff(self.rx_tone_hz) > 12 {
                let tx_band = egui::Rect::from_min_max(
                    egui::pos2(tx_x - channel_half_width, response.rect.top()),
                    egui::pos2(tx_x + channel_half_width, response.rect.bottom()),
                );
                ui.painter().rect_filled(
                    tx_band,
                    0.0,
                    Color32::from_rgba_unmultiplied(240, 150, 60, 32),
                );
            }

            ui.painter().line_segment(
                [
                    egui::pos2(rx_x, response.rect.top()),
                    egui::pos2(rx_x, response.rect.bottom()),
                ],
                egui::Stroke::new(1.5_f32, Color32::from_rgb(120, 220, 120)),
            );
            ui.painter().line_segment(
                [
                    egui::pos2(tx_x, response.rect.top()),
                    egui::pos2(tx_x, response.rect.bottom()),
                ],
                egui::Stroke::new(1.5_f32, Color32::from_rgb(220, 160, 80)),
            );

            ui.painter().text(
                egui::pos2(response.rect.left() + 6.0, response.rect.top() + 4.0),
                egui::Align2::LEFT_TOP,
                format!("RX {} Hz", self.rx_tone_hz),
                egui::TextStyle::Small.resolve(ui.style()),
                Color32::from_rgb(120, 220, 120),
            );
            ui.painter().text(
                egui::pos2(response.rect.left() + 6.0, response.rect.top() + 20.0),
                egui::Align2::LEFT_TOP,
                format!("TX {} Hz", self.tx_tone_hz),
                egui::TextStyle::Small.resolve(ui.style()),
                Color32::from_rgb(220, 160, 80),
            );
        }
        ui.label(format!(
            "Audio: {}  |  0\u{2013}{} Hz ({} {})  |  L-click RX / R-click TX",
            snapshot.audio_spectrum_status,
            bw_hz.min(AUDIO_MAX_FREQ_HZ),
            snapshot.mode,
            snapshot
                .filter
                .map(|f| format!("FIL{f}"))
                .unwrap_or_default(),
        ));
    }

    pub(in super::super) fn draw_band_controls(&self, ui: &mut egui::Ui, snapshot: &GuiState) {
        ui.heading(format!("Band / Filter ({})", self.workspace_mode.label()));
        ui.separator();

        let current_hz = snapshot.frequency_hz.unwrap_or(0);
        let band_plan = workspace_band_plan(self.workspace_mode);

        // Band buttons — 3 per row
        ui.horizontal(|ui| {
            for &(label, freq_hz) in band_plan {
                let on_band = (current_hz as i64 - freq_hz as i64).unsigned_abs() < 200_000;
                if ui
                    .add(
                        egui::Button::new(RichText::new(label).monospace().strong()).fill(
                            if on_band {
                                Color32::from_rgb(30, 80, 30)
                            } else {
                                Color32::from_gray(40)
                            },
                        ),
                    )
                    .on_hover_text(format!(
                        "{:.3} MHz  {}",
                        freq_hz as f64 / 1_000_000.0,
                        self.workspace_mode.label()
                    ))
                    .clicked()
                {
                    self.send_command(GuiCommand::TuneWorkspaceBand(freq_hz));
                }
            }
        });

        ui.add_space(4.0);

        // Filter selector
        let supports_filter = find_model(&self.config.radio.model)
            .is_some_and(|profile| matches!(profile.protocol, Protocol::IcomCiV { .. }));
        ui.horizontal(|ui| {
            ui.label(RichText::new("BW").strong());
            for fil in 1u8..=3 {
                let active = snapshot.filter == Some(fil);
                let label = format!("FIL{fil}");
                if ui
                    .add_enabled(
                        supports_filter,
                        egui::Button::new(RichText::new(&label).monospace()).fill(if active {
                            Color32::from_rgb(20, 60, 120)
                        } else {
                            Color32::from_gray(40)
                        }),
                    )
                    .clicked()
                {
                    self.send_command(GuiCommand::SetFilter(fil));
                }
            }
        });
    }
}
