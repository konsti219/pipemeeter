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

mod node;
pub use node::*;
mod routing_graph;
use routing_graph::*;
mod virtual_device;
pub use virtual_device::VIRTUAL_DEVICE_PREFIX;
use virtual_device::*;
mod device;
use device::*;
mod pod;
use pod::*;
mod port;
pub use port::*;
mod worker;
use worker::*;

type PwProxies = HashMap<u32, PwProxy>;
pub type PwState = HashMap<u32, PwObject>;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct PwClient {
    pub module_id: u32,
    pub application_name: String,
}

#[derive(Debug, Clone)]
pub struct PwDevice {
    pub factory_id: u32,
    pub client_id: u32,
    pub device_api: String,
    pub description: String,
    pub name: String,
    pub nick: String,
    pub media_class: String,
    pub routes: Vec<PwDeviceRoute>,
}

#[derive(Debug, Clone)]
pub struct PwFactory {
    pub name: String,
    pub type_name: String,
    pub module_id: u32,
}

#[derive(Debug, Clone)]
pub struct PwLink {
    pub client_id: u32,
    pub input_node: u32,
    pub input_port: u32,
    pub output_node: u32,
    pub output_port: u32,
    pub managed_by_pipemeeter: bool,
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

enum BackendCommand {
    SyncManagedVirtualDevices {
        names: Vec<String>,
        reply: mpsc::Sender<Result<()>>,
    },
    SetNodeVolume {
        node_id: u32,
        volume: f32,
        reply: mpsc::Sender<Result<()>>,
    },
    SyncRouting {
        links: Vec<DesiredNodeLink>,
        reply: mpsc::Sender<Result<()>>,
    },
    CleanupManagedObjects {
        reply: mpsc::Sender<Result<()>>,
    },
    Shutdown {
        reply: mpsc::Sender<Result<()>>,
    },
}

fn send_reply(reply: mpsc::Sender<Result<()>>, res: Result<()>) {
    if reply.send(res).is_err() {
        warn!("frontend reply channel dropped before worker could reply");
    }
}

pub struct PipewireBackend {
    pub objects: Arc<Mutex<PwState>>,

    command_tx: pw::channel::Sender<BackendCommand>,
    handle: Option<JoinHandle<Result<()>>>,
}

#[derive(Debug, Clone)]
pub struct NodeSummary {
    pub id: u32,
    pub name: String,
    pub description: Option<String>,
    pub category: PwNodeCategory,
}

impl PipewireBackend {
    pub fn new() -> Result<Self> {
        let objects = Arc::new(Mutex::new(HashMap::new()));
        let (command_tx, command_rx) = pw::channel::channel();
        let (ready_tx, ready_rx) = mpsc::channel();

        let handle = pipewire_worker(objects.clone(), command_rx, ready_tx);
        match ready_rx.recv_timeout(COMMAND_TIMEOUT) {
            Ok(Ok(())) => {}
            Ok(Err(err)) => return Err(err),
            Err(err) => {
                bail!("timed out waiting for PipeWire worker startup: {err}");
            }
        }

        Ok(Self {
            objects,
            command_tx,
            handle: Some(handle),
        })
    }

    fn request<F>(&self, build: F) -> Result<()>
    where
        F: FnOnce(mpsc::Sender<Result<()>>) -> BackendCommand,
    {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.command_tx
            .send(build(reply_tx))
            .map_err(|_| anyhow::anyhow!("failed to send command to PipeWire worker"))?;

        match reply_rx.recv_timeout(COMMAND_TIMEOUT) {
            Ok(res) => res,
            Err(err) => bail!("timed out waiting for PipeWire command completion: {err}"),
        }
    }

    pub fn sync_managed_virtual_devices(&self, names: Vec<String>) -> Result<()> {
        self.request(|reply| BackendCommand::SyncManagedVirtualDevices { names, reply })
    }

    pub fn set_node_volume(&self, node_id: u32, volume: f32) -> Result<()> {
        self.request(|reply| BackendCommand::SetNodeVolume {
            node_id,
            volume,
            reply,
        })
    }

    pub fn sync_routing(&self, links: Vec<DesiredNodeLink>) -> Result<()> {
        self.request(|reply| BackendCommand::SyncRouting { links, reply })
    }

    pub fn cleanup_managed_objects(&self) -> Result<()> {
        self.request(|reply| BackendCommand::CleanupManagedObjects { reply })
    }
}

impl Drop for PipewireBackend {
    fn drop(&mut self) {
        let (reply_tx, reply_rx) = mpsc::channel();
        let _ = self
            .command_tx
            .send(BackendCommand::Shutdown { reply: reply_tx });
        let _ = reply_rx.recv_timeout(Duration::from_millis(500));

        if let Some(handle) = self.handle.take() {
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(err)) => error!("PipeWire worker exited with error: {err}"),
                Err(_) => error!("PipeWire worker thread panicked"),
            }
        }
    }
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
