mod audio;
pub(super) mod cwdit_adapter;
pub(super) mod decode;
pub(super) mod radio;

pub(super) use audio::spawn_audio_spectrum_worker;

use eframe::egui;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

// All profile workers share one egui context. Coalesce redundant presentation
// requests while allowing every profile to keep decoding and updating state.
static LAST_GUI_REPAINT_MS: AtomicU64 = AtomicU64::new(0);

pub(super) fn request_gui_repaint(ctx: &Arc<OnceLock<egui::Context>>) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let previous = LAST_GUI_REPAINT_MS.load(Ordering::Relaxed);
    if now.saturating_sub(previous) < 66
        || LAST_GUI_REPAINT_MS
            .compare_exchange(previous, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
    {
        return;
    }
    if let Some(ctx) = ctx.get() {
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::{request_gui_repaint, LAST_GUI_REPAINT_MS};
    use std::sync::{Arc, OnceLock};

    #[test]
    fn requests_repaint_when_a_context_is_available() {
        LAST_GUI_REPAINT_MS.store(0, std::sync::atomic::Ordering::Relaxed);
        let context = Arc::new(OnceLock::new());
        assert!(context.set(eframe::egui::Context::default()).is_ok());

        request_gui_repaint(&context);
    }

    #[test]
    fn coalesces_a_second_repaint_request_within_the_interval() {
        LAST_GUI_REPAINT_MS.store(u64::MAX, std::sync::atomic::Ordering::Relaxed);
        let context = Arc::new(OnceLock::new());

        request_gui_repaint(&context);
    }
}
