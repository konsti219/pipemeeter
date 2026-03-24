use super::*;

#[derive(Debug, Clone)]
pub struct PwNode {
    pub id: u32,
    pub name: String,
    pub description: Option<String>,
    pub nick: Option<String>,
    pub media_class: Option<String>,
    pub category: PwNodeCategory,
    pub media_name: Option<String>,
    // pub factory_id: u32,
    // pub client_id: Option<u32>,
    // pub client_api: Option<String>,
    pub device_id: Option<u32>,
    pub process_binary: Option<String>,
    pub input_ports: u32,
    pub output_ports: u32,
    pub volume: [f32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PwNodeCategory {
    OutputDevice,
    InputDevice,
    PlaybackStream,
    RecordingStream,
    PipemeeterNode,
    PipemeeterMeter,
    Other,
}

impl PwNodeCategory {
    pub fn is_user_facing(&self) -> bool {
        matches!(
            self,
            PwNodeCategory::OutputDevice
                | PwNodeCategory::InputDevice
                | PwNodeCategory::PlaybackStream
                | PwNodeCategory::RecordingStream
        )
    }

    pub fn is_pipemeeter(&self) -> bool {
        matches!(
            self,
            PwNodeCategory::PipemeeterNode | PwNodeCategory::PipemeeterMeter
        )
    }
}

// Media Classes:
// - Stream/Input/Audio for monitors
// - Audio/Device for devices
// - Audio/Sink for sinks (wivrn)

/// Handle a new node being found in the pw graph.
/// Returns true if a graph rebuild shoudl happen.
pub(super) fn handle_node_global(
    global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>,
    props: &pw::spa::utils::dict::DictRef,
    objects: &mut PwState,
    objects_for_updates: &Arc<Mutex<PwState>>,
    registry: &pw::registry::RegistryRc,
    proxies: &Rc<RefCell<PwProxies>>,
) -> bool {
    let node_id = global.id;
    let objects_info = objects_for_updates.clone();
    let objects_param = objects_for_updates.clone();
    let proxy = registry.bind::<pw::node::Node, _>(global).unwrap();
    let listener = proxy
        .add_listener_local()
        .info(move |info| {
            let mut objects = objects_info.lock().unwrap();
            if let Some(PwObject::Node(node)) = objects.get_mut(&node_id) {
                node.input_ports = info.n_input_ports();
                node.output_ports = info.n_output_ports();

                let Some(props) = info.props() else {
                    return;
                };

                // Nodes do not loose properties and info updates only contain changed properties, so do merging

                if let Some(media_name) = props.get(&MEDIA_NAME).owned() {
                    node.media_name = Some(media_name);
                }

                if let Some(process_binary) = props.get(&APP_PROCESS_BINARY).owned() {
                    node.process_binary = Some(process_binary);
                }

                let media_class = props.get(&MEDIA_CLASS).owned();
                let media_class = media_class.or_else(|| node.media_class.clone());
                let monitor = props.get(&STREAM_MONITOR).map_or(false, |v| v == "true");

                // Only attempt to reclassify if it is not already detected as a special node.
                if node.category.is_user_facing() {
                    node.category =
                        classify_node_category(&node.name, media_class.as_deref(), monitor);
                }
            }
        })
        .param(move |_seq, _id, _index, _next, param| {
            let Some(param) = param else {
                return;
            };

            let mut objects = objects_param.lock().unwrap();
            if let Some(PwObject::Node(node)) = objects.get_mut(&node_id) {
                if let Some(volume) = node_volume_from_param(param) {
                    node.volume = volume;
                }
            }
        })
        .register();
    proxy.subscribe_params(&[ParamType::Props]);
    proxy.enum_params(1, Some(ParamType::Props), 0, u32::MAX);

    let name = props.get(&pw::keys::NODE_NAME).unwrap().to_owned();
    let media_class = props.get(&MEDIA_CLASS).owned();

    // It is importent to immediatly identify our own nodes as such, which why we fallback to
    // the name instead of a custom property.
    let category = classify_node_category(
        &name,
        media_class.as_deref(),
        props.get(&STREAM_MONITOR).is_some_and(|v| v == "true"),
    );

    let node = PwNode {
        id: global.id,
        name: name,
        description: props.get(&pw::keys::NODE_DESCRIPTION).owned(),
        nick: props.get(&pw::keys::NODE_NICK).owned(),
        media_class,
        category,
        media_name: None, // never in the static properties
        // factory_id: props.get(&FACTORY_ID).unwrap().parse::<u32>().unwrap(),
        // client_id: props.get(&CLIENT_ID).map(|v| v.parse::<u32>().unwrap()),
        // client_api: props.get(&CLIENT_API).map(ToOwned::to_owned),
        device_id: props.get(&DEVICE_ID).map(|v| v.parse::<u32>().unwrap()),
        process_binary: None,
        // default to max as we use this for checking if we know all ports of a node.
        // we update this in the info callback
        input_ports: u32::MAX,
        output_ports: u32::MAX,
        volume: [1.0, 1.0],
    };
    objects.insert(global.id, PwObject::Node(node));
    proxies
        .borrow_mut()
        .insert(global.id, PwProxy::Node(proxy, listener));

    category.is_user_facing()
}

fn classify_media_class(media_class: Option<&str>) -> PwNodeCategory {
    let Some(media_class) = media_class else {
        return PwNodeCategory::Other;
    };

    if media_class.starts_with("Audio/Sink") {
        PwNodeCategory::OutputDevice
    } else if media_class.starts_with("Audio/Source") {
        PwNodeCategory::InputDevice
    } else if media_class.starts_with("Stream/Output/Audio") {
        PwNodeCategory::PlaybackStream
    } else if media_class.starts_with("Stream/Input/Audio") {
        PwNodeCategory::RecordingStream
    } else {
        PwNodeCategory::Other
    }
}

fn classify_node_category(name: &str, media_class: Option<&str>, monitor: bool) -> PwNodeCategory {
    if name.starts_with(VIRTUAL_DEVICE_PREFIX) {
        if monitor {
            return PwNodeCategory::PipemeeterMeter;
        } else {
            return PwNodeCategory::PipemeeterNode;
        }
    }

    if monitor {
        return PwNodeCategory::Other;
    }

    classify_media_class(media_class)
}

pub(super) fn set_node_volume_impl(
    objects: &Arc<Mutex<PwState>>,
    proxies: &Rc<RefCell<PwProxies>>,
    node_id: u32,
    volume: f32,
) -> Result<()> {
    let volume = volume.max(0.0);
    let param_bytes = build_node_volume_props_param([volume, volume])?;
    let param = pw::spa::pod::Pod::from_bytes(&param_bytes)
        .context("failed to build props pod for volume command")?;

    info!(
        "issuing PipeWire command: set node volume id={} volume={:.3}",
        node_id, volume
    );

    let proxies_ref = proxies.borrow();
    let proxy = match proxies_ref.get(&node_id) {
        Some(PwProxy::Node(node, _listener)) => node,
        Some(_) => {
            bail!("object id={} is not a node", node_id);
        }
        None => {
            bail!("node id={} not found", node_id);
        }
    };

    proxy.set_param(ParamType::Props, 0, param);

    let (device_id, device_routes) = {
        let state = objects.lock().unwrap();
        let Some(PwObject::Node(node)) = state.get(&node_id) else {
            return Ok(());
        };
        let Some(device_id) = node.device_id else {
            return Ok(());
        };
        let Some(PwObject::Device(device)) = state.get(&device_id) else {
            return Ok(());
        };
        (device_id, device.routes.clone())
    };

    if device_routes.is_empty() {
        return Ok(());
    }

    let device_proxy = match proxies_ref.get(&device_id) {
        Some(PwProxy::Device(device, _listener)) => device,
        _ => {
            return Ok(());
        }
    };

    for route in device_routes {
        let route_param_bytes = build_device_route_volume_param(route, [volume, volume])?;
        let route_param = pw::spa::pod::Pod::from_bytes(&route_param_bytes)
            .context("failed to build route pod for volume command")?;
        device_proxy.set_param(ParamType::Route, 0, route_param);
    }

    Ok(())
}
