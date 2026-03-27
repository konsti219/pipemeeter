use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use log::{error, info, warn};
use pipewire::{self as pw};
use pw::keys::*;
use pw::properties::properties;
use pw::spa::param::ParamType;
use pw::types::ObjectType;

use crate::config::AppConfig;

mod device;
use device::*;
mod factory;
use factory::*;
mod meter;
use meter::*;
mod node;
pub use node::*;
mod node_resolution;
use node_resolution::*;
mod pod;
use pod::*;
mod port;
pub use port::*;
mod routing_graph;
use routing_graph::*;
mod virtual_device;
pub use virtual_device::VIRTUAL_DEVICE_PREFIX;
use virtual_device::*;
mod worker;
use worker::*;

type PwProxies = HashMap<u32, PwProxy>;
pub type PwState = HashMap<u32, PwObject>;

const COMMAND_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
pub struct PwClient {
    pub module_id: u32,
    pub application_name: String,
}

#[derive(Debug, Clone)]
pub struct PwLink {
    pub client_id: u32,
    pub input_node: u32,
    pub input_port: u32,
    pub output_node: u32,
    pub output_port: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DesiredNodeLink {
    pub output_node: u32,
    pub input_node: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PwMediaType {
    Audio,
    Video,
    Midi,
    Unknown,
}

#[derive(Debug, Clone)]
pub enum PwObject {
    Client(PwClient),
    // ClientEndpoint,
    // ClientNode,
    // ClientSession,
    Core,
    Device(PwDevice),
    // Endpoint,
    // EndpointLink,
    // EndpointStream,
    Factory(PwFactory),
    Link(PwLink),
    Metadata(String),
    Module(String),
    Node(PwNode),
    Port(PwPort),
    Profiler,
    // Registry,
    // Session,
}

enum PwProxy {
    Device(pw::device::Device, pw::device::DeviceListener),
    Node(pw::node::Node, pw::node::NodeListener),
    Port(pw::port::Port, pw::port::PortListener),
}

impl std::fmt::Debug for PwProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Device(_, _) => f.debug_struct("PwProxy::Device").finish(),
            Self::Node(_, _) => f.debug_struct("PwProxy::Node").finish(),
            Self::Port(_, _) => f.debug_struct("PwProxy::Port").finish(),
        }
    }
}

fn create_mainloop() -> Result<(pw::main_loop::MainLoopRc, pw::core::CoreRc)> {
    let mainloop =
        pw::main_loop::MainLoopRc::new(None).context("failed to create PipeWire loop")?;
    let context = pw::context::ContextRc::new(&mainloop, None)
        .context("failed to create PipeWire context")?;
    let core = context
        .connect_rc(None)
        .context("failed to connect to PipeWire core")?;
    Ok((mainloop, core))
}

#[derive(Debug)]
enum BackendCommand {
    UpdateRouting,
    SetNodeVolume {
        node_id: u32,
        volume: f32,
    },
    Shutdown {
        reply: mpsc::Sender<Result<()>>,
    },
    /// For backend internal use
    ResetTimer,
}

fn send_reply(reply: mpsc::Sender<Result<()>>, res: Result<()>) {
    if reply.send(res).is_err() {
        warn!("frontend reply channel dropped before worker could reply");
    }
}

pub struct PipewireBackend {
    pub objects: Arc<Mutex<PwState>>,
    meters: Arc<Mutex<HashMap<u32, [f32; 2]>>>,

    command_tx: pw::channel::Sender<BackendCommand>,
    handle: Option<JoinHandle<Result<()>>>,
}

impl PipewireBackend {
    pub fn new(config: Arc<Mutex<AppConfig>>) -> Result<Self> {
        let objects = Arc::new(Mutex::new(HashMap::new()));
        let meters = Arc::new(Mutex::new(HashMap::<u32, [f32; 2]>::new()));
        let (command_tx, command_rx) = pw::channel::channel();
        let (ready_tx, ready_rx) = mpsc::channel();

        let handle = pipewire_worker(
            config,
            objects.clone(),
            meters.clone(),
            command_tx.clone(),
            command_rx,
            ready_tx,
        );
        match ready_rx.recv_timeout(COMMAND_TIMEOUT) {
            Ok(Ok(())) => {}
            Ok(Err(err)) => return Err(err),
            Err(err) => {
                bail!("timed out waiting for PipeWire worker startup: {err}");
            }
        }

        Ok(Self {
            objects,
            meters,
            command_tx,
            handle: Some(handle),
        })
    }

    pub fn set_node_volume(&self, node_id: u32, volume: f32) -> Result<()> {
        self.command_tx
            .send(BackendCommand::SetNodeVolume { node_id, volume })
            .map_err(|_| anyhow::anyhow!("failed to send command to PipeWire worker"))
    }

    pub fn update_routing(&self) -> Result<()> {
        self.command_tx
            .send(BackendCommand::UpdateRouting)
            .map_err(|_| anyhow::anyhow!("failed to send command to PipeWire worker"))
    }

    pub fn node_peak_meter(&self, node_id: u32) -> [f32; 2] {
        let meters = self.meters.lock().unwrap();
        meters.get(&node_id).copied().unwrap_or_default()
    }
}

impl Drop for PipewireBackend {
    fn drop(&mut self) {
        let (reply_tx, reply_rx) = mpsc::channel();
        let _ = self
            .command_tx
            .send(BackendCommand::Shutdown { reply: reply_tx });
        let shutdown_ack = reply_rx.recv_timeout(Duration::from_millis(500)).is_ok();

        if let Some(handle) = self.handle.take() {
            if shutdown_ack {
                match handle.join() {
                    Ok(Ok(())) => {}
                    Ok(Err(err)) => error!("PipeWire worker exited with error: {err}"),
                    Err(_) => error!("PipeWire worker thread panicked"),
                }
            } else {
                // Worker did not acknowledge shutdown quickly enough.
                // Drop the join handle to detach instead of blocking app exit.
                warn!("PipeWire worker did not acknowledge shutdown in time; detaching thread");
            }
        }
    }
}

pub fn virtual_input_combined_name(index: usize) -> String {
    format!("{VIRTUAL_DEVICE_PREFIX}vin-{}", index + 1)
}

pub fn virtual_output_combined_name(index: usize) -> String {
    format!("{VIRTUAL_DEVICE_PREFIX}vout-{}", index + 1)
}

pub trait PwStateExt {
    fn nodes(&self) -> impl Iterator<Item = &PwNode>;
}

impl PwStateExt for PwState {
    fn nodes(&self) -> impl Iterator<Item = &PwNode> {
        self.values().filter_map(move |obj| {
            let PwObject::Node(node) = obj else {
                return None;
            };
            Some(node)
        })
    }
}

trait OptionExt {
    fn owned(&self) -> Option<String>;
}

impl OptionExt for Option<&str> {
    fn owned(&self) -> Option<String> {
        self.map(|s| s.to_owned())
    }
}
