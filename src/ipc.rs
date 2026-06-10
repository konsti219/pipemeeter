//! Unix-socket control interface for external clients (e.g. the wayvr watch).
//!
//! The protocol is line-delimited JSON: one request object per line, one
//! response object per line. Volume values are the 0.0..=1.0 slider value
//! (identical to `StripConfig::volume` and the GUI slider); the perceptual
//! cubic curve is applied downstream when the value reaches PipeWire.
//!
//! Only virtual input strips are addressable, because those are the strips
//! whose configured volume the worker actively enforces onto a combined node
//! (see `sync_virtual_input_combined_volumes`). Setting a volume here updates
//! the shared config and triggers a routing reconcile so the change is pushed
//! into PipeWire and reflected in the GUI.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::Result;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};

use crate::config::{AppConfig, save_config};
use crate::pipewire_backend::RoutingTrigger;

#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Request {
    ListStrips,
    GetVolume { strip: String },
    SetVolume { strip: String, volume: f32 },
}

#[derive(Debug, Default, Serialize)]
struct Response {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    volume: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    strips: Option<Vec<String>>,
}

impl Response {
    fn ok() -> Self {
        Self {
            ok: true,
            ..Default::default()
        }
    }

    fn err(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(message.into()),
            ..Default::default()
        }
    }
}

pub fn socket_path() -> PathBuf {
    let base = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
    base.join("pipemeeter.sock")
}

pub fn spawn_control_socket(
    config: Arc<Mutex<AppConfig>>,
    config_path: PathBuf,
    routing: RoutingTrigger,
) {
    let path = socket_path();
    // A leftover socket file from a previous run would make bind() fail with
    // EADDRINUSE even though nobody is listening, so clear it first.
    let _ = std::fs::remove_file(&path);

    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(err) => {
            error!("failed to bind control socket at {}: {err}", path.display());
            return;
        }
    };
    info!("control socket listening at {}", path.display());

    thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let config = config.clone();
                    let config_path = config_path.clone();
                    let routing = routing.clone();
                    thread::spawn(move || {
                        if let Err(err) = handle_client(stream, config, config_path, routing) {
                            warn!("control socket client disconnected: {err}");
                        }
                    });
                }
                Err(err) => warn!("control socket accept error: {err}"),
            }
        }
    });
}

fn handle_client(
    stream: UnixStream,
    config: Arc<Mutex<AppConfig>>,
    config_path: PathBuf,
    routing: RoutingTrigger,
) -> Result<()> {
    let reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => handle_request(request, &config, &config_path, &routing),
            Err(err) => Response::err(format!("invalid request: {err}")),
        };

        let mut bytes = serde_json::to_vec(&response)?;
        bytes.push(b'\n');
        writer.write_all(&bytes)?;
        writer.flush()?;
    }

    Ok(())
}

fn handle_request(
    request: Request,
    config: &Arc<Mutex<AppConfig>>,
    config_path: &PathBuf,
    routing: &RoutingTrigger,
) -> Response {
    match request {
        Request::ListStrips => {
            let config = config.lock().unwrap();
            let strips = config
                .virtual_inputs
                .iter()
                .map(|strip| strip.name.clone())
                .collect();
            Response {
                strips: Some(strips),
                ..Response::ok()
            }
        }
        Request::GetVolume { strip } => {
            let config = config.lock().unwrap();
            match config.virtual_inputs.iter().find(|s| s.name == strip) {
                Some(found) => Response {
                    volume: Some(found.volume),
                    ..Response::ok()
                },
                None => Response::err(format!("no such virtual input strip: {strip}")),
            }
        }
        Request::SetVolume { strip, volume } => {
            let volume = volume.clamp(0.0, 1.0);
            let snapshot = {
                let mut config = config.lock().unwrap();
                match config.virtual_inputs.iter_mut().find(|s| s.name == strip) {
                    Some(found) => found.volume = volume,
                    None => return Response::err(format!("no such virtual input strip: {strip}")),
                }
                config.clone()
            };

            // Persist so the change survives a restart, matching the GUI slider.
            if let Err(err) = save_config(config_path, &snapshot) {
                warn!("failed to persist config from control socket: {err}");
            }

            // Push the new volume into PipeWire via the worker's reconcile path.
            if let Err(err) = routing.trigger() {
                return Response::err(format!("failed to trigger routing update: {err}"));
            }

            Response::ok()
        }
    }
}
