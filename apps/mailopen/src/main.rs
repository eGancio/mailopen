//! mailopen: a small mail client with a unified inbox. GUI in egui; everything else in
//! `mailopen-core`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("mailopen")
            .with_inner_size([1240.0, 780.0])
            .with_min_inner_size([900.0, 560.0])
            .with_icon(mailopen::icon::window_icon()),
        ..Default::default()
    };
    #[cfg(windows)]
    prefer_dx12(&mut options);
    eframe::run_native(
        "mailopen",
        options,
        Box::new(|cc| Ok(Box::new(mailopen::app::MailApp::new(cc)))),
    )
}

/// On Windows wgpu would pick Vulkan first, and some Intel Vulkan drivers (Iris Xe, 30.0.x)
/// segfault right after adapter selection. DX12 is native and stable there.
/// `WGPU_BACKEND` still overrides this.
#[cfg(windows)]
fn prefer_dx12(options: &mut eframe::NativeOptions) {
    use eframe::egui_wgpu::{WgpuSetup, wgpu};
    if wgpu::Backends::from_env().is_some() {
        return;
    }
    if let WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
        setup.instance_descriptor.backends = wgpu::Backends::DX12;
    }
}
