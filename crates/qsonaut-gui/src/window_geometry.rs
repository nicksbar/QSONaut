use eframe::egui;
use serde::{Deserialize, Serialize};
use tracing::info;

use super::{WINDOW_DEFAULT_SIZE, WINDOW_GEOMETRY_FILE, WINDOW_MIN_SIZE};

const WINDOW_MAX_RESTORE_SIZE: [f32; 2] = [8_192.0, 8_192.0];
const WINDOW_MAX_RESTORE_COORDINATE: f32 = 16_384.0;

/// Native window geometry restored by QSONaut instead of eframe. Applying it to
/// the `ViewportBuilder` means winit configures the window once, while it is
/// still hidden, instead of showing and re-hiding it for each late change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct WindowGeometry {
    #[serde(default)]
    pub(super) maximized: bool,
    #[serde(default)]
    position: Option<[f32; 2]>,
    #[serde(default)]
    size: Option<[f32; 2]>,
}

impl WindowGeometry {
    pub(super) fn path() -> std::path::PathBuf {
        qsonaut_log::app_config_dir().join(WINDOW_GEOMETRY_FILE)
    }

    pub(super) fn load() -> Option<Self> {
        let path = Self::path();
        let raw = std::fs::read_to_string(&path).ok()?;
        match serde_json::from_str::<Self>(&raw) {
            Ok(geometry) => {
                let geometry = geometry.sanitized();
                info!(path = %path.display(), ?geometry, "Loaded window geometry");
                Some(geometry)
            }
            Err(error) => {
                info!(%error, "Ignoring unreadable window geometry");
                None
            }
        }
    }

    /// A stale profile can carry a monitor that no longer exists or values from
    /// a crashed session, which would otherwise open the window off-screen.
    fn sanitized(mut self) -> Self {
        self.size = self
            .size
            .filter(|s| s.iter().all(|v| v.is_finite()))
            .map(|s| {
                [
                    s[0].clamp(WINDOW_MIN_SIZE[0], WINDOW_MAX_RESTORE_SIZE[0]),
                    s[1].clamp(WINDOW_MIN_SIZE[1], WINDOW_MAX_RESTORE_SIZE[1]),
                ]
            });
        self.position = self.position.filter(|p| {
            p.iter()
                .all(|v| v.is_finite() && v.abs() <= WINDOW_MAX_RESTORE_COORDINATE)
        });

        // A maximized viewport without a remembered unmaximized size cannot be
        // safely restored. This is common after a WSLg/X11 display disconnect:
        // winit reports maximized=true but never supplies usable bounds. Fall
        // back to the normal window size so the next launch remains visible.
        if self.maximized && self.size.is_none() {
            info!("Ignoring maximized window geometry without a usable saved size");
            self.maximized = false;
            self.size = Some(WINDOW_DEFAULT_SIZE);
        }
        self
    }

    pub(super) fn read(ctx: &egui::Context, previous: Option<Self>) -> Option<Self> {
        ctx.input(|input| {
            let viewport = input.viewport();
            let maximized = viewport.maximized.unwrap_or(false);
            // Restore bounds are meaningless while maximized, so keep the last
            // known un-maximized rect instead of overwriting it.
            if maximized {
                let previous = previous.unwrap_or_default();
                return Some(Self {
                    maximized: true,
                    position: previous.position,
                    size: previous.size,
                });
            }
            let position = viewport.outer_rect.map(|rect| [rect.min.x, rect.min.y])?;
            let size = viewport
                .inner_rect
                .map(|rect| [rect.width(), rect.height()])?;
            Some(Self {
                maximized: false,
                position: Some(position),
                size: Some(size),
            })
        })
    }

    pub(super) fn save(self) {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&self) {
            Ok(json) => {
                if let Err(error) = std::fs::write(&path, json) {
                    info!(%error, path = %path.display(), "Failed to save window geometry");
                }
            }
            Err(error) => info!(%error, "Failed to serialize window geometry"),
        }
    }

    pub(super) fn apply(self, mut builder: egui::ViewportBuilder) -> egui::ViewportBuilder {
        if let Some(size) = self.size {
            builder = builder.with_inner_size(size);
        }
        if let Some(position) = self.position {
            builder = builder.with_position(position);
        }
        // Maximized is deliberately not set here: winit would `SW_MAXIMIZE` the
        // still-unpainted window and immediately `SW_HIDE` it again, which is
        // the white flash. It is applied after the first frame instead.
        builder
    }
}

#[cfg(test)]
mod tests {
    use super::WindowGeometry;

    #[test]
    fn maximized_geometry_without_bounds_falls_back_to_visible_window() {
        let geometry = WindowGeometry {
            maximized: true,
            position: None,
            size: None,
        }
        .sanitized();

        assert!(!geometry.maximized);
        assert_eq!(geometry.size, Some(super::WINDOW_DEFAULT_SIZE));
    }

    #[test]
    fn geometry_bounds_are_clamped() {
        let geometry = WindowGeometry {
            maximized: false,
            position: Some([20_000.0, f32::NAN]),
            size: Some([50_000.0, 100.0]),
        }
        .sanitized();

        assert_eq!(geometry.position, None);
        assert_eq!(geometry.size, Some([8_192.0, 680.0]));
    }
}
