use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortDirection {
    In,
    Out,
}

#[derive(Debug, Clone)]
pub struct PwPort {
    pub node_id: u32,
    pub category: PwNodeCategory,
    pub port_id: u32,
    pub name: String,
    pub direction: PortDirection,
    pub format_dsp: Option<String>,
    pub audio_channel: Option<String>,
    pub media_type: PwMediaType,
    pub monitor: bool,
}

pub(super) fn handle_port_global(
    global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>,
    props: &pw::spa::utils::dict::DictRef,
    objects: &mut PwState,
    objects_for_updates: &Arc<Mutex<PwState>>,
    registry: &pw::registry::RegistryRc,
    proxies: &Rc<RefCell<PwProxies>>,
) -> bool {
    let object_id = global.id;
    let objects_for_port_info = objects_for_updates.clone();

    let proxy = registry.bind::<pw::port::Port, _>(global).unwrap();
    let listener = proxy
        .add_listener_local()
        .param(move |_seq, id, _index, _next, param| {
            if id != ParamType::EnumFormat {
                return;
            }

            let Some(param) = param else {
                return;
            };

            let media_type = media_type_from_enum_format(param);

            let mut objects = objects_for_port_info.lock().unwrap();
            if let Some(PwObject::Port(port)) = objects.get_mut(&object_id) {
                port.media_type = media_type;
            }
        })
        .register();

    proxy.subscribe_params(&[ParamType::EnumFormat]);
    proxy.enum_params(1, Some(ParamType::EnumFormat), 0, u32::MAX);

    let node_id = props.get(&NODE_ID).unwrap().parse::<u32>().unwrap();
    let port_id = props.get(&PORT_ID).unwrap().parse::<u32>().unwrap();
    let name = props.get(&PORT_NAME).unwrap().to_owned();
    let direction = match props.get(&PORT_DIRECTION).unwrap() {
        "in" => PortDirection::In,
        "out" => PortDirection::Out,
        value => unreachable!("unexpected port direction: {value}"),
    };
    let format_dsp = props.get(&FORMAT_DSP).owned();
    let audio_channel = props.get(&AUDIO_CHANNEL).owned();
    let monitor = props
        .get(&PORT_MONITOR)
        .map(|v| v == "true")
        .unwrap_or(false);

    let category = match objects.get(&node_id).unwrap() {
        PwObject::Node(node) => node.category,
        _ => unreachable!("port's node is not a node"),
    };

    objects.insert(
        global.id,
        PwObject::Port(PwPort {
            node_id,
            category,
            port_id,
            name,
            direction,
            format_dsp,
            audio_channel,
            media_type: PwMediaType::Unknown,
            monitor,
        }),
    );
    proxies
        .borrow_mut()
        .insert(global.id, PwProxy::Port(proxy, listener));

    !matches!(category, PwNodeCategory::Other)
}
