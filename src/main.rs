use anyhow::Context;
use app::PipeMeeterApp;
use eframe::egui;
use log::error;

mod app;
mod config;
mod pipewire_backend;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    let icon = load_app_icon()?;
    let app = PipeMeeterApp::new();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("pipemeeter")
            .with_inner_size(app.desired_viewport_size())
            .with_icon(icon),
        ..Default::default()
    };
    let run_res = eframe::run_native("Pipemeeter", options, Box::new(|_cc| Ok(Box::new(app))));

    if let Err(err) = run_res {
        error!("failed to run egui app: {err}");
    }

    Ok(())
}

fn load_app_icon() -> anyhow::Result<egui::IconData> {
    let bytes = include_bytes!("pipemeeter.png");
    let image = image::load_from_memory(bytes).context("failed to decode pipemeeter.png")?;
    let image = image.into_rgba8();
    let (width, height) = image.dimensions();

    Ok(egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}
