use std::collections::{HashMap, HashSet};

use super::*;

pub const MANAGED_VIRTUAL_STRIP_PREFIX: &str = "pm-combined-";

fn normalized_prefixed(name: &str) -> &str {
    name.strip_prefix("pipemeeter/").unwrap_or(name)
}

fn combined_name_from_node(node: &PwNode) -> Option<String> {
    let candidates = [
        Some(node.name.as_str()),
        node.description.as_deref(),
        node.nick.as_deref(),
        node.managed_device_name.as_deref(),
    ];

    candidates.into_iter().flatten().find_map(|value| {
        let normalized = normalized_prefixed(value);
        if normalized.starts_with(MANAGED_VIRTUAL_STRIP_PREFIX) {
            Some(normalized.to_owned())
        } else {
            None
        }
    })
}

fn destroy_nodes_by_id(
    registry: &pw::registry::RegistryRc,
    ids: impl IntoIterator<Item = u32>,
    reason: &str,
) -> Result<()> {
    for id in ids {
        info!("graph change: destroy node id={} reason='{}'", id, reason);
        registry
            .destroy_global(id)
            .into_result()
            .with_context(|| format!("failed to destroy node id={} ({})", id, reason))?;
    }
    Ok(())
}

pub fn create_virtual_device_impl(core: &pw::core::CoreRc, name: &str) -> Result<()> {
    let node_factory = "adapter";
    let name = format!("pipemeeter/{}", name);

    info!(
        "graph change: create virtual node name='{}' node_factory='{}'",
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
                "media.class" => "Audio/Sink",
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

pub fn sync_managed_virtual_devices_impl(
    core: &pw::core::CoreRc,
    registry: &pw::registry::RegistryRc,
    objects: &Arc<Mutex<PwState>>,
    desired_names: &[String],
) -> Result<()> {
    let desired_set = desired_names.iter().cloned().collect::<HashSet<_>>();

    let existing_by_name = {
        let state = objects.lock().unwrap();
        let mut by_name = HashMap::<String, Vec<u32>>::new();

        for (id, obj) in state.iter() {
            let PwObject::Node(node) = obj else {
                continue;
            };

            let Some(name) = combined_name_from_node(node) else {
                continue;
            };

            by_name.entry(name).or_default().push(*id);
        }

        by_name
    };

    for (name, ids) in &existing_by_name {
        if !desired_set.contains(name) {
            destroy_nodes_by_id(registry, ids.iter().copied(), "no longer desired")?;
        }
    }

    for name in &desired_set {
        match existing_by_name.get(name) {
            None => {
                create_virtual_device_impl(core, name)?;
            }
            Some(ids) if ids.len() > 1 => {
                destroy_nodes_by_id(registry, ids.iter().copied(), "deduplicate managed nodes")?;
                create_virtual_device_impl(core, name)?;
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
                PwObject::Node(node) if combined_name_from_node(node).is_some() => Some(*id),
                _ => None,
            })
            .collect::<Vec<_>>()
    };

    destroy_nodes_by_id(registry, candidate_ids, "shutdown cleanup")
}
