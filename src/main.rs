#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod glass;
mod model;
mod settings;
mod terminal;
mod window_shape;

use eframe::egui;
use model::Store;

fn main() -> anyhow::Result<()> {
    let store = Store::new()?;
    let settings = store.load_settings().unwrap_or_default();
    let icon = image::load_from_memory(include_bytes!("../assets/bmux.png"))?.into_rgba8();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("[bmux]")
        .with_app_id("dev.bmux.app")
        .with_drag_and_drop(true)
        .with_inner_size(settings.initial_size())
        .with_min_inner_size([640.0, 400.0])
        .with_decorations(false)
        .with_transparent(true)
        .with_icon(egui::IconData {
            width: icon.width(),
            height: icon.height(),
            rgba: icon.into_raw(),
        });
    if settings.remember_frame
        && let Ok(bytes) = std::fs::read(store.data_dir().join("window.json"))
        && let Ok(frame) = serde_json::from_slice::<app::WindowFrame>(&bytes)
    {
        viewport = viewport.with_inner_size(frame.size);
        if let Some(pos) = frame.position {
            viewport = viewport.with_position(pos);
        }
    }
    let options = eframe::NativeOptions {
        viewport,
        vsync: true,
        ..Default::default()
    };
    eframe::run_native(
        "[bmux]",
        options,
        Box::new(move |cc| Ok(Box::new(app::BmuxApp::new(cc, store)))),
    )
    .map_err(|err| anyhow::anyhow!("Unable to open Bmux: {err}"))
}
