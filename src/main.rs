use anyhow::Context;
use app::PipeMeeterApp;
use eframe::egui;
use log::{error, info};

mod app;
mod config;
mod ipc;
mod pipewire_backend;
mod session;
mod volume;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    // Register with the session manager so logout/shutdown quits the app cleanly
    // regardless of window state (minimized, occluded, or visible). A minimized
    // Wayland window is never sent `close` by the compositor, so without this the
    // app would block logout; here the session manager tells us to quit directly.
    if let Err(err) = session::connect() {
        info!("session management unavailable: {err}");
    }

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

    // Exit the process directly instead of unwinding `main`: dropping eframe tears
    // down egui-winit's clipboard, whose worker thread races destroying Wayland
    // proxies on shutdown and intermittently segfaults or deadlocks (reported by
    // the compositor as "not responding"). Nothing needs unwinding — the config
    // is persisted eagerly on edit — so terminate immediately.
    std::process::exit(0);
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
