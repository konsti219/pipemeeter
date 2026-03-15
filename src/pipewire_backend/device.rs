use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PwDeviceRoute {
    pub index: u32,
    pub direction: u32,
    pub device: u32,
}

pub(super) fn handle_device_global(
    global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>,
    props: &pw::spa::utils::dict::DictRef,
    objects: &mut PwState,
    objects_for_updates: &Arc<Mutex<PwState>>,
    registry: &pw::registry::RegistryRc,
    proxies: &Rc<RefCell<PwProxies>>,
) {
    let object_id = global.id;
    let objects_for_param = objects_for_updates.clone();

    let proxy = registry.bind::<pw::device::Device, _>(global).unwrap();
    let listener = proxy
        .add_listener_local()
        .param(move |_seq, id, _index, _next, param| {
            if id != ParamType::Route {
                return;
            }
            let Some(param) = param else {
                return;
            };
            let Some(route) = route_descriptor_from_param(param) else {
                return;
            };

            let mut objects = objects_for_param.lock().unwrap();
            if let Some(PwObject::Device(device)) = objects.get_mut(&object_id) {
                if let Some(existing) = device
                    .routes
                    .iter_mut()
                    .find(|r| r.index == route.index && r.direction == route.direction)
                {
                    *existing = route;
                } else {
                    device.routes.push(route);
                }
            }
        })
        .register();

    proxy.subscribe_params(&[ParamType::Route]);
    proxy.enum_params(1, Some(ParamType::Route), 0, u32::MAX);

    let factory_id = props.get(&FACTORY_ID).unwrap().parse::<u32>().unwrap();
    let client_id = props.get(&CLIENT_ID).unwrap().parse::<u32>().unwrap();
    let device_api = props.get(&DEVICE_API).unwrap().to_owned();
    let description = props.get(&DEVICE_DESCRIPTION).unwrap().to_owned();
    let name = props.get(&DEVICE_NAME).unwrap().to_owned();
    let nick = props.get(&DEVICE_NICK).unwrap().to_owned();
    let media_class = props.get(&MEDIA_CLASS).unwrap().to_owned();

    objects.insert(
        global.id,
        PwObject::Device(PwDevice {
            factory_id,
            client_id,
            device_api,
            description,
            name,
            nick,
            media_class,
            routes: Vec::new(),
        }),
    );

    proxies
        .borrow_mut()
        .insert(global.id, PwProxy::Device(proxy, listener));
}
