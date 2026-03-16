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
    pub volume: [f32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PwNodeCategory {
    OutputDevice,
    InputDevice,
    PlaybackStream,
    RecordingStream,
    Other,
}

// Media Classes:
// - Stream/Input/Audio for monitors
// - Audio/Device for devices
// - Audio/Sink for sinks (wivrn)

pub(super) fn handle_node_global(
    global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>,
    props: &pw::spa::utils::dict::DictRef,
    objects: &mut PwState,
    objects_for_updates: &Arc<Mutex<PwState>>,
    registry: &pw::registry::RegistryRc,
    proxies: &Rc<RefCell<PwProxies>>,
) {
    let node_id = global.id;
    let objects_info = objects_for_updates.clone();
    let objects_param = objects_for_updates.clone();
    let proxy = registry.bind::<pw::node::Node, _>(global).unwrap();
    let listener = proxy
        .add_listener_local()
        .info(move |info| {
            let media_name = info.props().and_then(|p| p.get(&MEDIA_NAME)).owned();
            let monitor = info
                .props()
                .and_then(|p| p.get(&STREAM_MONITOR))
                .map_or(false, |v| v == "true");
            let process_binary = info
                .props()
                .and_then(|p| p.get(&APP_PROCESS_BINARY))
                .map(ToOwned::to_owned);

            let mut objects = objects_info.lock().unwrap();
            if let Some(PwObject::Node(node)) = objects.get_mut(&node_id) {
                let media_class = info.props().and_then(|p| p.get(&MEDIA_CLASS)).owned();
                let media_class = media_class.or_else(|| node.media_class.clone());

                // only set if present. This could theoratically lead to stale data, but in practice there are more cases
                // where it is missing in some callbacks.
                if let Some(media_name) = media_name {
                    node.media_name = Some(media_name);
                }

                if !monitor {
                    node.category = classify_media_class(media_class.as_deref());
                } else {
                    node.category = PwNodeCategory::Other;
                }

                if let Some(process_binary) = process_binary {
                    node.process_binary = Some(process_binary);
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

    let node = PwNode {
        id: global.id,
        name: props.get(&pw::keys::NODE_NAME).unwrap().to_owned(),
        description: props.get(&pw::keys::NODE_DESCRIPTION).owned(),
        nick: props.get(&pw::keys::NODE_NICK).owned(),
        media_class: props.get(&MEDIA_CLASS).owned(),
        category: classify_media_class(props.get(&MEDIA_CLASS)),
        media_name: None, // never in the static properties
        // factory_id: props.get(&FACTORY_ID).unwrap().parse::<u32>().unwrap(),
        // client_id: props.get(&CLIENT_ID).map(|v| v.parse::<u32>().unwrap()),
        // client_api: props.get(&CLIENT_API).map(ToOwned::to_owned),
        device_id: props.get(&DEVICE_ID).map(|v| v.parse::<u32>().unwrap()),
        process_binary: None,
        volume: [1.0, 1.0],
    };
    objects.insert(global.id, PwObject::Node(node));
    proxies
        .borrow_mut()
        .insert(global.id, PwProxy::Node(proxy, listener));
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

pub fn create_virtual_device_impl(core: &pw::core::CoreRc, name: &str) -> Result<()> {
    let node_factory = "adapter";
    let name = format!("pipemeeter/{}", name);

    info!(
        "issuing PipeWire command: create virtual device name='{}' node_factory='{}'",
        name, node_factory
    );

    let _node = core
        .create_object::<pw::node::Node>(
            node_factory,
            &properties! {
                "factory.name" => "support.null-audio-sink",
                "node.name" => name.as_str(),
                "node.description" => name.as_str(),
                "media.type" => "Audio",
                "media.class" => "Audio/Duplex/Virtual",
                "audio.channels" => "2",
                "audio.position" => "FL FR",
                "monitor.channel-volumes" => "true",
                "object.linger" => "true",
                "pipemeeter.managed" => "true",
                "pipemeeter.device-name" => name.as_str(),
            },
        )
        .context("failed to create virtual device")?;

    Ok(())
}

pub fn node_matches_virtual_device(node: &PwNode, name: &str) -> bool {
    let prefixed_name = format!("pipemeeter/{}", name);

    node.name == name
        || node.name == prefixed_name
        || node.description.as_deref() == Some(name)
        || node.nick.as_deref() == Some(name)
}

pub fn remove_virtual_device_impl(
    registry: &pw::registry::RegistryRc,
    objects: &Arc<Mutex<PwState>>,
    name: &str,
) -> Result<()> {
    let candidate_ids = {
        let state = objects.lock().unwrap();
        state
            .iter()
            .filter_map(|(id, obj)| match obj {
                PwObject::Node(node) if node_matches_virtual_device(node, name) => Some(*id),
                _ => None,
            })
            .collect::<Vec<_>>()
    };

    if candidate_ids.is_empty() {
        return Ok(());
    }

    let mut removed_any = false;
    for id in candidate_ids {
        registry
            .destroy_global(id)
            .into_result()
            .with_context(|| format!("failed to destroy node id={}", id))?;
        removed_any = true;
    }

    if !removed_any {
        bail!("virtual device not found in PipeWire");
    }

    Ok(())
}
