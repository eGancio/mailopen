//! The app icon for the windows (taskbar, title bar, Alt+Tab). Drawn once, in code.

use std::sync::{Arc, OnceLock};

/// The icon as egui window icon data (256 × 256).
pub fn window_icon() -> Arc<egui::IconData> {
    static ICON: OnceLock<Arc<egui::IconData>> = OnceLock::new();
    Arc::clone(ICON.get_or_init(|| {
        Arc::new(egui::IconData {
            rgba: crate::icon_art::icon_rgba(256),
            width: 256,
            height: 256,
        })
    }))
}
