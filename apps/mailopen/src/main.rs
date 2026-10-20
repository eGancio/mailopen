//! mailopen: a small mail client with a unified inbox. GUI in egui; everything else in
//! `mailopen-core`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("mailopen")
            .with_inner_size([1240.0, 780.0])
            .with_min_inner_size([900.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "mailopen",
        options,
        Box::new(|cc| Ok(Box::new(mailopen::app::MailApp::new(cc)))),
    )
}
