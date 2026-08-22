use super::super::*;
use crate::visuals::build_audio_waterfall_image_with_theme;

impl QsonautGuiApp {
    pub(in super::super) fn draw_radio_waterfall(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        snapshot: &GuiState,
    ) {
        let (rows, source_revision, source_bins, render_bins) = if self.radio_scope_view
            == RadioScopeView::Narrow
        {
            if self.radio_scope_lock_if_to_filter {
                self.radio_scope_span_code = scope_span_for_filter(&snapshot.mode, snapshot.filter);
            }
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
            let response = ui
                .add(egui::Image::new((tex.id(), display_size)).sense(egui::Sense::click()))
                .on_hover_text("Click a signal to tune the radio VFO to that point");
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
            if response.clicked() {
                if let Some(pos) = response.interact_pointer_pos() {
                    let rel =
                        ((pos.x - response.rect.left()) / response.rect.width()).clamp(0.0, 1.0);
                    let target = match self.radio_scope_view {
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
                            low.saturating_add(
                                ((high.saturating_sub(low)) as f32 * rel).round() as u64
                            )
                        }),
                        RadioScopeView::Overview => snapshot.frequency_hz.and_then(|frequency| {
                            band_edges_for_frequency(Some(frequency)).map(|(low, high, _)| {
                                low.saturating_add(
                                    ((high.saturating_sub(low)) as f32 * rel).round() as u64,
                                )
                            })
                        }),
                    };
                    if let Some(target) = target {
                        self.send_command(GuiCommand::TuneTo(target));
                        self.profile_io_status = format!(
                            "Scope tune requested: {:.6} MHz",
                            target as f64 / 1_000_000.0
                        );
                    }
                }
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

        let filter_bw_hz = filter_bandwidth_hz(&snapshot.mode, snapshot.filter);
        let is_cw = self.workspace_mode == WorkspaceMode::Cw;
        let is_sstv = self.workspace_mode == WorkspaceMode::Sstv;
        let bw_hz = filter_bw_hz;
        let sstv_display_offset_hz = if snapshot.sstv_auto_target {
            snapshot
                .sstv_locked_offset_hz
                .unwrap_or(self.sstv_tuning_offset_hz)
        } else {
            self.sstv_tuning_offset_hz
        };
        let rx_cursor_hz = if is_sstv {
            (1_100_i32 + sstv_display_offset_hz).max(0) as u32
        } else if is_cw {
            u32::from(self.cw_tone_hz)
        } else {
            self.rx_tone_hz
        };
        let tx_cursor_hz = if is_sstv {
            rx_cursor_hz
        } else if is_cw {
            u32::from(self.cw_tone_hz)
        } else {
            self.tx_tone_hz
        };
        let channel_hz = if is_cw {
            80
        } else {
            match self.workspace_mode {
                WorkspaceMode::Ft8 => 50,
                WorkspaceMode::Ft4 => 90,
                WorkspaceMode::Fst4 => 70,
                WorkspaceMode::Wspr => 6,
                WorkspaceMode::Jt9 => 16,
                WorkspaceMode::Jt65 | WorkspaceMode::Q65 => 180,
                WorkspaceMode::Sstv => 1_200,
                _ => 50,
            }
        };
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
            let image = build_audio_waterfall_image_with_theme(
                &snapshot.audio_waterfall_rows,
                bw_hz,
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
                    if is_sstv {
                        let minimum_center_hz = 700_i32;
                        let maximum_center_hz = (capped_bw as i32 - 600).max(minimum_center_hz);
                        let selected_center_hz =
                            (pick_hz as i32).clamp(minimum_center_hz, maximum_center_hz);
                        self.sstv_tuning_offset_hz = selected_center_hz - 1_700;
                        self.sstv_auto_target = false;
                        self.profile_io_status = format!(
                            "SSTV manual target: {}–{} Hz ({:+} Hz)",
                            1_100_i32 + self.sstv_tuning_offset_hz,
                            2_300_i32 + self.sstv_tuning_offset_hz,
                            self.sstv_tuning_offset_hz,
                        );
                        let mut shared = self.state.lock().expect("ui state lock poisoned");
                        shared.sstv_auto_target = false;
                        shared.sstv_tuning_offset_hz = self.sstv_tuning_offset_hz;
                        shared.sstv_locked_offset_hz = None;
                        shared.sstv_progress = None;
                        shared.record_sstv_rx_event(format!(
                            "MANUAL TARGET clicked at {:+} Hz · window {}–{} Hz",
                            self.sstv_tuning_offset_hz,
                            1_100_i32 + self.sstv_tuning_offset_hz,
                            2_300_i32 + self.sstv_tuning_offset_hz,
                        ));
                        tracing::info!(
                            offset_hz = self.sstv_tuning_offset_hz,
                            window_low_hz = 1_100_i32 + self.sstv_tuning_offset_hz,
                            window_high_hz = 2_300_i32 + self.sstv_tuning_offset_hz,
                            "SSTV manual target selected from audio waterfall"
                        );
                    } else {
                        let selected_hz = if is_cw {
                            pick_hz.clamp(200, 3_000)
                        } else {
                            pick_hz.saturating_sub(channel_hz / 2)
                        };
                        if is_cw {
                            self.cw_tone_hz = selected_hz as u16;
                        }
                        self.rx_tone_hz = selected_hz;
                        if is_cw || !self.ft8_hold_tx_freq {
                            self.tx_tone_hz = selected_hz;
                        }
                        self.profile_dirty = true;
                        self.persist_profile("Auto-saved");
                        self.profile_io_status =
                            format!("RX audio cursor set: {} Hz", self.rx_tone_hz);
                    }
                }
                if response.secondary_clicked() && !is_sstv {
                    let selected_hz = if is_cw {
                        pick_hz.clamp(200, 3_000)
                    } else {
                        pick_hz.saturating_sub(channel_hz / 2)
                    };
                    if is_cw {
                        self.cw_tone_hz = selected_hz as u16;
                        self.rx_tone_hz = selected_hz;
                    }
                    self.tx_tone_hz = selected_hz;
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                    self.profile_io_status = format!("TX tone set: {} Hz", self.tx_tone_hz);
                }
            }

            let bw = bw_hz.clamp(1, AUDIO_MAX_FREQ_HZ) as f32;
            let rx_x = response.rect.left()
                + (rx_cursor_hz.min(bw as u32) as f32 / bw) * response.rect.width();
            let tx_x = response.rect.left()
                + (tx_cursor_hz.min(bw as u32) as f32 / bw) * response.rect.width();

            let channel_half_width =
                (channel_hz as f32 / bw * response.rect.width() / 2.0).max(2.0);
            let rx_band = if is_cw {
                egui::Rect::from_min_max(
                    egui::pos2(rx_x - channel_half_width, response.rect.top()),
                    egui::pos2(rx_x + channel_half_width, response.rect.bottom()),
                )
            } else {
                egui::Rect::from_min_max(
                    egui::pos2(rx_x, response.rect.top()),
                    egui::pos2(rx_x + channel_half_width * 2.0, response.rect.bottom()),
                )
            };
            ui.painter().rect_filled(
                rx_band,
                0.0,
                Color32::from_rgba_unmultiplied(80, 220, 110, 32),
            );
            let tx_band = if is_cw {
                egui::Rect::from_min_max(
                    egui::pos2(tx_x - channel_half_width, response.rect.top()),
                    egui::pos2(tx_x + channel_half_width, response.rect.bottom()),
                )
            } else {
                egui::Rect::from_min_max(
                    egui::pos2(tx_x, response.rect.top()),
                    egui::pos2(tx_x + channel_half_width * 2.0, response.rect.bottom()),
                )
            };
            if !is_sstv {
                ui.painter().rect_filled(
                    tx_band,
                    0.0,
                    Color32::from_rgba_unmultiplied(240, 150, 60, 28),
                );
            }

            ui.painter().line_segment(
                [
                    egui::pos2(rx_x, response.rect.top()),
                    egui::pos2(rx_x, response.rect.bottom()),
                ],
                egui::Stroke::new(1.5_f32, Color32::from_rgb(120, 220, 120)),
            );
            if !is_cw {
                let edge_x = rx_x
                    + if is_sstv {
                        channel_half_width * 2.0
                    } else {
                        channel_half_width
                    };
                ui.painter().line_segment(
                    [
                        egui::pos2(edge_x, response.rect.top()),
                        egui::pos2(edge_x, response.rect.bottom()),
                    ],
                    egui::Stroke::new(1.0_f32, Color32::from_rgb(120, 220, 120)),
                );
            }
            if !is_sstv {
                ui.painter().line_segment(
                    [
                        egui::pos2(tx_x, response.rect.top()),
                        egui::pos2(tx_x, response.rect.bottom()),
                    ],
                    egui::Stroke::new(1.5_f32, Color32::from_rgb(220, 160, 80)),
                );
            }

            ui.painter().text(
                egui::pos2(response.rect.left() + 6.0, response.rect.top() + 4.0),
                egui::Align2::LEFT_TOP,
                if is_sstv {
                    format!(
                        "SSTV RX {}–{} Hz · {}",
                        rx_cursor_hz,
                        rx_cursor_hz + channel_hz,
                        if snapshot.sstv_auto_target {
                            if snapshot.sstv_locked_offset_hz.is_some() {
                                "AUTO LOCK"
                            } else {
                                "AUTO SCAN"
                            }
                        } else {
                            "MANUAL"
                        }
                    )
                } else {
                    format!(
                        "{} RX {} Hz",
                        if is_cw { "CW CENTER" } else { "RX EDGE" },
                        rx_cursor_hz
                    )
                },
                egui::TextStyle::Small.resolve(ui.style()),
                Color32::from_rgb(120, 220, 120),
            );
            ui.painter().text(
                egui::pos2(response.rect.left() + 6.0, response.rect.top() + 20.0),
                egui::Align2::LEFT_TOP,
                if is_sstv {
                    format!(
                        "Sync {} · pixels {}–{} Hz · offset {:+} Hz",
                        1_200_i32 + sstv_display_offset_hz,
                        1_500_i32 + sstv_display_offset_hz,
                        2_300_i32 + sstv_display_offset_hz,
                        sstv_display_offset_hz,
                    )
                } else {
                    format!("TX {} Hz", self.tx_tone_hz)
                },
                egui::TextStyle::Small.resolve(ui.style()),
                Color32::from_rgb(220, 160, 80),
            );
        }
        if is_sstv {
            ui.label(format!(
                "Audio: {}  |  radio passband 0–{} Hz · SSTV decoder {}–{} Hz · {} ({} {})",
                snapshot.audio_spectrum_status,
                bw_hz.min(AUDIO_MAX_FREQ_HZ),
                rx_cursor_hz,
                rx_cursor_hz + channel_hz,
                if snapshot.sstv_auto_target {
                    if snapshot.sstv_locked_offset_hz.is_some() {
                        "auto-target locked; click to override"
                    } else {
                        "auto-target scanning; click to override"
                    }
                } else {
                    "manual target; enable Auto Target to scan"
                },
                snapshot.mode,
                snapshot
                    .filter
                    .map(|f| format!("FIL{f}"))
                    .unwrap_or_default(),
            ));
        } else {
            ui.label(format!(
                "Audio: {}  |  0\u{2013}{} Hz · channel {} Hz · selected level {}/255 · {} ({} {})  |  L-click RX / R-click TX",
                snapshot.audio_spectrum_status,
                bw_hz.min(AUDIO_MAX_FREQ_HZ),
                channel_hz,
                audio_cursor_level(&snapshot.audio_waterfall_rows, rx_cursor_hz),
                if is_cw {
                    "CW tone search ±120 Hz"
                } else {
                    "digital channel edge"
                },
                snapshot.mode,
                snapshot
                    .filter
                    .map(|f| format!("FIL{f}"))
                    .unwrap_or_default(),
            ));
        }
    }
}
