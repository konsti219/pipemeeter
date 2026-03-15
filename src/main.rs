mod app;
mod config;
mod pipewire_backend;
mod ui;

use app::PipeMeeterApp;
use eframe::egui;
use log::error;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let app = PipeMeeterApp::new();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size(app.desired_viewport_size()),
        ..Default::default()
    };
    let run_res = eframe::run_native("Pipemeeter", options, Box::new(|_cc| Ok(Box::new(app))));

    if let Err(err) = run_res {
        error!("failed to run egui app: {err}");
    }

    Ok(())
}
