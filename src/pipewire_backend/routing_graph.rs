use std::collections::{HashMap, HashSet};

use super::*;

const METER_TAP_NODE_PREFIX: &str = "pipemeeter/meter-";

fn managed_virtual_strip_nodes(state: &PwState) -> HashSet<u32> {
    state
        .values()
        .filter_map(|obj| {
            let PwObject::Node(node) = obj else {
                return None;
            };

            if node.category == PwNodeCategory::Pipemeeter
                && (node.name.starts_with("pipemeeter/vin-")
                    || node.name.starts_with("pipemeeter/vout-"))
            {
                Some(node.id)
            } else {
                None
            }
        })
        .collect()
}

fn audio_ports_by_node(
    state: &PwState,
    direction: PortDirection,
    monitor_output_nodes: &HashSet<u32>,
) -> HashMap<u32, Vec<(u32, u32)>> {
    let mut by_node: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();

    for (global_id, obj) in state {
        let PwObject::Port(port) = obj else {
            continue;
        };

        if port.direction != direction || port.media_type != PwMediaType::Audio {
            continue;
        }

        if port.monitor
            && !(direction == PortDirection::Out && monitor_output_nodes.contains(&port.node_id))
        {
            continue;
        }

        by_node
            .entry(port.node_id)
            .or_default()
            .push((*global_id, port.port_id));
    }

    for ports in by_node.values_mut() {
        ports.sort_by_key(|(_, port_id)| *port_id);
    }

    by_node
}

fn infer_managed_client_id(state: &PwState) -> Option<u32> {
    let app_name = env!("CARGO_PKG_NAME").to_ascii_lowercase();

    state.iter().find_map(|(id, obj)| {
        let PwObject::Client(client) = obj else {
            return None;
        };

        let candidate = client.application_name.to_ascii_lowercase();
        if candidate == app_name || candidate.contains(&app_name) {
            Some(*id)
        } else {
            None
        }
    })
}

fn desired_meter_tap_node_links(state: &PwState) -> Vec<DesiredNodeLink> {
    state
        .values()
        .filter_map(|obj| {
            let PwObject::Node(node) = obj else {
                return None;
            };

            if node.category != PwNodeCategory::Pipemeeter {
                return None;
            }

            let target_node_id = node
                .name
                .strip_prefix(METER_TAP_NODE_PREFIX)
                .and_then(|value| value.parse::<u32>().ok())?;

            if !state.contains_key(&target_node_id) {
                return None;
            }

            Some(DesiredNodeLink {
                output_node: target_node_id,
                input_node: node.id,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct DesiredPortLink {
    output_node: u32,
    output_port: u32,
    input_node: u32,
    input_port: u32,
}

pub fn remove_managed_links_impl(
    registry: &pw::registry::RegistryRc,
    objects: &Arc<Mutex<PwState>>,
) -> Result<()> {
    let candidate_ids = {
        let state = objects.lock().unwrap();
        state
            .iter()
            .filter_map(|(id, obj)| match obj {
                PwObject::Link(link) if link.managed_by_pipemeeter => Some(*id),
                _ => None,
            })
            .collect::<Vec<_>>()
    };

    for id in candidate_ids {
        info!(
            "graph change: destroy link id={} reason='shutdown cleanup'",
            id
        );
        registry
            .destroy_global(id)
            .into_result()
            .with_context(|| format!("failed to destroy managed link id={}", id))?;
    }

    Ok(())
}

pub fn sync_routing_impl(
    core: &pw::core::CoreRc,
    registry: &pw::registry::RegistryRc,
    objects: &Arc<Mutex<PwState>>,
    desired_node_links: &[DesiredNodeLink],
) -> Result<()> {
    let (desired_port_links, links_to_remove, managed_client_id) = {
        let state = objects.lock().unwrap();
        let managed_client_id = infer_managed_client_id(&state)
            .expect("pipemeeter PipeWire client.id not found while creating managed links");

        let node_category = state
            .values()
            .filter_map(|obj| {
                let PwObject::Node(node) = obj else {
                    return None;
                };
                Some((node.id, node.category))
            })
            .collect::<HashMap<_, _>>();

        let monitor_output_nodes = managed_virtual_strip_nodes(&state);

        let out_ports_by_node =
            audio_ports_by_node(&state, PortDirection::Out, &monitor_output_nodes);
        let in_ports_by_node =
            audio_ports_by_node(&state, PortDirection::In, &monitor_output_nodes);

        let mut all_desired_node_links = desired_node_links.to_vec();
        all_desired_node_links.extend(desired_meter_tap_node_links(&state));

        let mut desired_port_links = HashSet::new();
        for link in &all_desired_node_links {
            let out_category = node_category
                .get(&link.output_node)
                .copied()
                .unwrap_or(PwNodeCategory::Other);
            let in_category = node_category
                .get(&link.input_node)
                .copied()
                .unwrap_or(PwNodeCategory::Other);

            if out_category == PwNodeCategory::Other || in_category == PwNodeCategory::Other {
                continue;
            }

            let Some(out_ports) = out_ports_by_node.get(&link.output_node) else {
                continue;
            };
            let Some(in_ports) = in_ports_by_node.get(&link.input_node) else {
                continue;
            };

            for ((out_global_id, _), (in_global_id, _)) in out_ports.iter().zip(in_ports.iter()) {
                desired_port_links.insert(DesiredPortLink {
                    output_node: link.output_node,
                    output_port: *out_global_id,
                    input_node: link.input_node,
                    input_port: *in_global_id,
                });
            }
        }

        let mut links_to_remove = Vec::new();
        let mut existing_port_links = HashSet::new();

        for (global_id, obj) in state.iter() {
            let PwObject::Link(link) = obj else {
                continue;
            };

            let in_category = node_category
                .get(&link.input_node)
                .copied()
                .unwrap_or(PwNodeCategory::Other);
            let out_category = node_category
                .get(&link.output_node)
                .copied()
                .unwrap_or(PwNodeCategory::Other);

            if in_category == PwNodeCategory::Other || out_category == PwNodeCategory::Other {
                continue;
            }

            let key = DesiredPortLink {
                output_node: link.output_node,
                output_port: link.output_port,
                input_node: link.input_node,
                input_port: link.input_port,
            };
            existing_port_links.insert(key);
            if !desired_port_links.contains(&key) {
                links_to_remove.push(*global_id);
            }
        }

        (
            desired_port_links,
            (links_to_remove, existing_port_links),
            managed_client_id,
        )
    };

    let (links_to_remove, existing_port_links) = links_to_remove;

    for link_id in links_to_remove {
        info!(
            "graph change: destroy link id={} reason='routing diff'",
            link_id
        );
        registry
            .destroy_global(link_id)
            .into_result()
            .with_context(|| format!("failed to destroy link id={}", link_id))?;
    }

    for link in desired_port_links {
        if existing_port_links.contains(&link) {
            continue;
        }

        let client_id_text = managed_client_id.to_string();
        let out_node_text = link.output_node.to_string();
        let in_node_text = link.input_node.to_string();
        let out_port_text = link.output_port.to_string();
        let in_port_text = link.input_port.to_string();

        info!(
            "graph change: creating link output_node={} output_port={} input_node={} input_port={} client_id={}",
            link.output_node, link.output_port, link.input_node, link.input_port, managed_client_id
        );

        let _link = core
            .create_object::<pw::link::Link>(
                "link-factory",
                &properties! {
                    "client.id" => client_id_text.as_str(),
                    "link.output.node" => out_node_text.as_str(),
                    "link.output.port" => out_port_text.as_str(),
                    "link.input.node" => in_node_text.as_str(),
                    "link.input.port" => in_port_text.as_str(),
                    "object.linger" => "true",
                    "pipemeeter.managed" => "true",
                },
            )
            .with_context(|| {
                format!(
                    "failed to create managed link output_node={} output_port={} input_node={} input_port={}",
                    link.output_node, link.output_port, link.input_node, link.input_port
                )
            })?;
    }

    Ok(())
}
