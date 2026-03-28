use std::sync::OnceLock;

use super::*;

pub(super) fn handle_factory_global(
    global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>,
    props: &pw::spa::utils::dict::DictRef,
    objects: &mut PwState,
) {
    let name = props.get("factory.name").unwrap().to_owned();
    let type_name = props.get("factory.type.name").unwrap().to_owned();

    if type_name == "PipeWire:Interface:Node" && name.starts_with("adapter") {
        ADAPTER_FACTORY_NAME.set(name.clone()).unwrap();
    } else if type_name == "PipeWire:Interface:Link" {
        LINK_FACTORY_NAME.set(name.clone()).unwrap();
    }

    objects.insert(global.id, PwObject::Factory);
}

pub(super) static ADAPTER_FACTORY_NAME: OnceLock<String> = OnceLock::new();
pub(super) static LINK_FACTORY_NAME: OnceLock<String> = OnceLock::new();
