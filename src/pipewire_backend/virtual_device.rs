use std::collections::HashMap;

use super::*;
use crate::config::AppConfig;

pub const VIRTUAL_DEVICE_PREFIX: &str = "pipemeeter/";

fn is_virtual_device_name(name: &str) -> bool {
    name.starts_with("pipemeeter/vin-") || name.starts_with("pipemeeter/vout-")
}

fn destroy_nodes_by_id(
    registry: &pw::registry::RegistryRc,
    ids: impl IntoIterator<Item = u32>,
    reason: &str,
) -> Result<()> {
    for id in ids {
        info!("graph change: destroy node id={id} reason='{reason}'",);
        registry
            .destroy_global(id)
            .into_result()
            .with_context(|| format!("failed to destroy node id={id} ({reason})"))?;
    }
    Ok(())
}

static MAX_NODES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn create_virtual_device_impl(
    core: &pw::core::CoreRc,
    name: &str,
    media_class: &str,
) -> Result<()> {
    info!("graph change: create virtual node name='{name}' media.class='{media_class}'");

    let current = MAX_NODES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if current >= 16 {
        warn!(
            "graph change: create virtual node name='{name}' failed: too many nodes (current={current})"
        );
        return Ok(());
    }

    let _node = core
        .create_object::<pw::node::Node>(
            ADAPTER_FACTORY_NAME.get().unwrap(),
            &properties! {
                "factory.name" => "support.null-audio-sink",
                "node.name" => name,
                "node.description" => name,
                "media.type" => "Audio",
                "media.class" => media_class,
                "node.virtual" => "true",
                "device.class" => "filter",
                "audio.channels" => "2",
                "audio.position" => "FL FR",
                "monitor.channel-volumes" => "true",
                "object.linger" => "true",
            },
        )
        .context("failed to create virtual device")?;

    Ok(())
}

pub fn sync_managed_virtual_devices_impl(
    core: &pw::core::CoreRc,
    registry: &pw::registry::RegistryRc,
    objects: &Arc<Mutex<PwState>>,
    config: &AppConfig,
) -> Result<()> {
    let desired = (0..config.virtual_inputs.len())
        .map(|i| (virtual_input_combined_name(i), "Audio/Sink/Virtual"))
        .chain(
            (0..config.virtual_outputs.len())
                .map(|i| (virtual_output_combined_name(i), "Audio/Source/Virtual")),
        )
        .collect::<Vec<_>>();

    let existing_by_name = {
        let state = objects.lock().unwrap();
        let mut by_name = HashMap::<String, Vec<u32>>::new();

        for (id, obj) in state.iter() {
            let PwObject::Node(node) = obj else {
                continue;
            };

            if !node.category.is_pipemeeter() {
                continue;
            }

            if !is_virtual_device_name(&node.name) {
                continue;
            };

            by_name.entry(node.name.clone()).or_default().push(*id);
        }

        by_name
    };

    for (name, ids) in &existing_by_name {
        if !desired.iter().any(|(desired_name, _)| desired_name == name) {
            destroy_nodes_by_id(registry, ids.iter().copied(), "no longer desired")?;
        }
    }

    for (name, media_class) in &desired {
        match existing_by_name.get(name) {
            None => {
                create_virtual_device_impl(core, name, media_class)?;
            }
            Some(ids) if ids.len() > 1 => {
                destroy_nodes_by_id(registry, ids.iter().copied(), "deduplicate managed nodes")?;
                create_virtual_device_impl(core, name, media_class)?;
            }
            Some(_) => {}
        }
    }

    Ok(())
}

pub fn remove_managed_virtual_devices_impl(
    registry: &pw::registry::RegistryRc,
    objects: &Arc<Mutex<PwState>>,
) -> Result<()> {
    let candidate_ids = {
        let state = objects.lock().unwrap();
        state
            .iter()
            .filter_map(|(id, obj)| match obj {
                PwObject::Node(node) if node.category.is_pipemeeter() => Some(*id),
                _ => None,
            })
            .collect::<Vec<_>>()
    };

    destroy_nodes_by_id(registry, candidate_ids, "shutdown cleanup")
}
