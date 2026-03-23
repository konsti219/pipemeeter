use std::sync::OnceLock;

use super::*;

#[derive(Debug, Clone)]
pub struct PwFactory {
    pub name: String,
    pub type_name: String,
    pub module_id: u32,
}

pub(super) fn handle_factory_global(
    global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>,
    props: &pw::spa::utils::dict::DictRef,
    objects: &mut PwState,
) {
    let name = props.get("factory.name").unwrap().to_owned();
    let type_name = props.get("factory.type.name").unwrap().to_owned();
    let module_id = props.get("module.id").unwrap().parse::<u32>().unwrap();

    if type_name == "PipeWire:Interface:Node" && name.starts_with("adapter") {
        ADAPTER_FACTORY_NAME.set(name.clone()).unwrap();
    } else if type_name == "PipeWire:Interface:Link" {
        LINK_FACTORY_NAME.set(name.clone()).unwrap();
    }

    objects.insert(
        global.id,
        PwObject::Factory(PwFactory {
            name,
            type_name,
            module_id,
        }),
    );
}

pub(super) static ADAPTER_FACTORY_NAME: OnceLock<String> = OnceLock::new();
pub(super) static LINK_FACTORY_NAME: OnceLock<String> = OnceLock::new();
