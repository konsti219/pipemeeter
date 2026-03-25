use super::*;
use crate::config::{AppConfig, NodeMatchProperty, NodeMatchRequirement, StripConfig};
use glob::Pattern;
use std::cell::Cell;
use std::collections::HashSet;

#[derive(Clone, Copy)]
struct ResolvedSetRef<'a> {
    physical_inputs: &'a [Vec<u32>],
    virtual_inputs: &'a [Vec<u32>],
    physical_outputs: &'a [Vec<u32>],
    virtual_outputs: &'a [Vec<u32>],
}

struct ResolvedSet {
    physical_inputs: Vec<Vec<u32>>,
    virtual_inputs: Vec<Vec<u32>>,
    physical_outputs: Vec<Vec<u32>>,
    virtual_outputs: Vec<Vec<u32>>,
}

#[derive(Clone, Copy)]
enum ResolvedGroup {
    PhysicalInput,
    VirtualInput,
    PhysicalOutput,
    VirtualOutput,
}

fn virtual_input_combined_name(index: usize) -> String {
    format!("{VIRTUAL_DEVICE_PREFIX}vin-{}", index + 1)
}

fn virtual_output_combined_name(index: usize) -> String {
    format!("{VIRTUAL_DEVICE_PREFIX}vout-{}", index + 1)
}

fn human_slider_to_pipewire_linear(human_slider: f32) -> f32 {
    let clamped = human_slider.clamp(0.0, 1.0);
    clamped * clamped * clamped
}

fn managed_virtual_strip_names(config: &AppConfig) -> Vec<String> {
    let mut names = Vec::new();

    for i in 0..config.virtual_inputs.len() {
        names.push(virtual_input_combined_name(i));
    }

    for i in 0..config.virtual_outputs.len() {
        names.push(virtual_output_combined_name(i));
    }

    names
}

fn managed_node_id(state: &PwState, managed_name: &str) -> Option<u32> {
    state.values().find_map(|obj| {
        let PwObject::Node(node) = obj else {
            return None;
        };

        if node.name == managed_name {
            Some(node.id)
        } else {
            None
        }
    })
}

fn node_match_value<'a>(node: &'a PwNode, match_property: NodeMatchProperty) -> Option<&'a str> {
    let val = match match_property {
        NodeMatchProperty::Name => Some(node.name.as_str()),
        NodeMatchProperty::Description => node.description.as_deref(),
        NodeMatchProperty::MediaName => node.media_name.as_deref(),
        NodeMatchProperty::ProcessBinary => node.process_binary.as_deref(),
    };
    val.map(str::trim).filter(|value| !value.is_empty())
}

fn requirement_matches_node(node: &PwNode, requirement: &NodeMatchRequirement) -> bool {
    let value = match node_match_value(node, requirement.match_property) {
        Some(value) => value,
        None => return false,
    };

    let pattern = requirement.pattern.trim();
    if pattern.is_empty() {
        return false;
    }

    match Pattern::new(pattern) {
        Ok(glob_pattern) => glob_pattern.matches(value),
        Err(_) => false,
    }
}

fn resolve_physical_ids(
    assigned_nodes: &mut HashSet<u32>,
    nodes: &[&PwNode],
    strips: &[StripConfig],
    category: PwNodeCategory,
) -> Vec<Vec<u32>> {
    let mut out = vec![Vec::new(); strips.len()];

    for (index, strip) in strips.iter().enumerate() {
        let requirements = strip.requirements.as_slice();
        if requirements.is_empty() {
            continue;
        }

        let mut candidates = nodes
            .iter()
            .copied()
            .filter(|node| !assigned_nodes.contains(&node.id))
            .filter(|node| {
                requirements
                    .iter()
                    .all(|requirement| requirement_matches_node(node, requirement))
            });

        if let Some(node) = candidates.find(|node| node.category == category) {
            assigned_nodes.insert(node.id);
            out[index].push(node.id);
            continue;
        }

        if strip.match_only_category {
            continue;
        }

        if let Some(node) = candidates.next() {
            assigned_nodes.insert(node.id);
            out[index].push(node.id);
        }
    }

    out
}

fn resolve_virtual_ids(
    assigned_nodes: &mut HashSet<u32>,
    nodes: &[&PwNode],
    strips: &[StripConfig],
    category: PwNodeCategory,
) -> Vec<Vec<u32>> {
    let mut out = vec![Vec::new(); strips.len()];

    for (index, strip) in strips
        .iter()
        .enumerate()
        .filter(|(_, strip)| !strip.requirements.is_empty())
    {
        let ids = nodes
            .iter()
            .copied()
            .filter(|node| !assigned_nodes.contains(&node.id))
            .filter(|node| {
                strip
                    .requirements
                    .iter()
                    .all(|requirement| requirement_matches_node(node, requirement))
            })
            .filter(|node| !strip.match_only_category || node.category == category)
            .map(|node| node.id)
            .collect::<Vec<_>>();

        for id in &ids {
            assigned_nodes.insert(*id);
        }

        out[index] = ids;
    }

    for (index, _strip) in strips
        .iter()
        .enumerate()
        .filter(|(_, strip)| strip.requirements.is_empty())
    {
        let ids = nodes
            .iter()
            .copied()
            .filter(|node| !assigned_nodes.contains(&node.id))
            .filter(|node| node.category == category)
            .map(|node| node.id)
            .collect::<Vec<_>>();

        for id in &ids {
            assigned_nodes.insert(*id);
        }

        out[index] = ids;
    }

    out
}

fn resolve_nodes_for_config(config: &AppConfig, state: &PwState) -> ResolvedSet {
    let mut nodes = state
        .nodes()
        .filter(|node| !node.category.is_pipemeeter())
        .collect::<Vec<_>>();
    nodes.sort_by_key(|node| node.id);

    let mut assigned_node_ids = HashSet::new();

    let physical_inputs = resolve_physical_ids(
        &mut assigned_node_ids,
        &nodes,
        &config.physical_inputs,
        PwNodeCategory::InputDevice,
    );

    let physical_outputs = resolve_physical_ids(
        &mut assigned_node_ids,
        &nodes,
        &config.physical_outputs,
        PwNodeCategory::OutputDevice,
    );

    let virtual_outputs = resolve_virtual_ids(
        &mut assigned_node_ids,
        &nodes,
        &config.virtual_outputs,
        PwNodeCategory::RecordingStream,
    );

    let virtual_inputs = resolve_virtual_ids(
        &mut assigned_node_ids,
        &nodes,
        &config.virtual_inputs,
        PwNodeCategory::PlaybackStream,
    );

    ResolvedSet {
        physical_inputs,
        virtual_inputs,
        physical_outputs,
        virtual_outputs,
    }
}

fn resolved_ids_for(resolved: ResolvedSetRef<'_>, group: ResolvedGroup, index: usize) -> Vec<u32> {
    match group {
        ResolvedGroup::PhysicalInput => resolved
            .physical_inputs
            .get(index)
            .cloned()
            .unwrap_or_default(),
        ResolvedGroup::VirtualInput => resolved
            .virtual_inputs
            .get(index)
            .cloned()
            .unwrap_or_default(),
        ResolvedGroup::PhysicalOutput => resolved
            .physical_outputs
            .get(index)
            .cloned()
            .unwrap_or_default(),
        ResolvedGroup::VirtualOutput => resolved
            .virtual_outputs
            .get(index)
            .cloned()
            .unwrap_or_default(),
    }
}

fn meter_target_node_names(
    config: &AppConfig,
    resolved: ResolvedSetRef<'_>,
    state: &PwState,
) -> Vec<String> {
    let id_to_name = state
        .values()
        .filter_map(|obj| {
            let PwObject::Node(node) = obj else {
                return None;
            };
            Some((node.id, node.name.clone()))
        })
        .collect::<HashMap<_, _>>();

    let mut names = HashSet::new();

    for name in managed_virtual_strip_names(config) {
        names.insert(name);
    }

    for index in 0..config.physical_inputs.len() {
        for node_id in resolved_ids_for(resolved, ResolvedGroup::PhysicalInput, index) {
            if let Some(name) = id_to_name.get(&node_id) {
                names.insert(name.clone());
            }
        }
    }

    for index in 0..config.physical_outputs.len() {
        for node_id in resolved_ids_for(resolved, ResolvedGroup::PhysicalOutput, index) {
            if let Some(name) = id_to_name.get(&node_id) {
                names.insert(name.clone());
            }
        }
    }

    names.into_iter().collect()
}

fn output_target_nodes_for_route(
    config: &AppConfig,
    resolved: ResolvedSetRef<'_>,
    route_index: usize,
    virtual_output_combined_ids: &[Option<u32>],
) -> Vec<u32> {
    if route_index < config.physical_outputs.len() {
        resolved_ids_for(resolved, ResolvedGroup::PhysicalOutput, route_index)
    } else {
        let virtual_index = route_index - config.physical_outputs.len();
        virtual_output_combined_ids
            .get(virtual_index)
            .copied()
            .flatten()
            .into_iter()
            .collect()
    }
}

fn desired_routing_links(
    config: &AppConfig,
    resolved: ResolvedSetRef<'_>,
    state: &PwState,
) -> Vec<DesiredNodeLink> {
    let virtual_input_combined_ids = (0..config.virtual_inputs.len())
        .map(|index| managed_node_id(state, &virtual_input_combined_name(index)))
        .collect::<Vec<_>>();
    let virtual_output_combined_ids = (0..config.virtual_outputs.len())
        .map(|index| managed_node_id(state, &virtual_output_combined_name(index)))
        .collect::<Vec<_>>();

    let mut desired = HashSet::new();

    for (index, combined_node_id) in virtual_input_combined_ids.iter().enumerate() {
        let Some(combined_node_id) = combined_node_id else {
            continue;
        };

        for source_node_id in resolved_ids_for(resolved, ResolvedGroup::VirtualInput, index) {
            desired.insert(DesiredNodeLink {
                output_node: source_node_id,
                input_node: *combined_node_id,
            });
        }
    }

    for (index, strip) in config.physical_inputs.iter().enumerate() {
        let source_nodes = resolved_ids_for(resolved, ResolvedGroup::PhysicalInput, index);

        for (route_index, enabled) in strip.routes_to_outputs.iter().copied().enumerate() {
            if !enabled {
                continue;
            }

            let target_nodes = output_target_nodes_for_route(
                config,
                resolved,
                route_index,
                &virtual_output_combined_ids,
            );

            for output_node in &source_nodes {
                for input_node in &target_nodes {
                    if output_node == input_node {
                        continue;
                    }

                    desired.insert(DesiredNodeLink {
                        output_node: *output_node,
                        input_node: *input_node,
                    });
                }
            }
        }
    }

    for (index, strip) in config.virtual_inputs.iter().enumerate() {
        let Some(source_node) = virtual_input_combined_ids.get(index).copied().flatten() else {
            continue;
        };

        for (route_index, enabled) in strip.routes_to_outputs.iter().copied().enumerate() {
            if !enabled {
                continue;
            }

            let target_nodes = output_target_nodes_for_route(
                config,
                resolved,
                route_index,
                &virtual_output_combined_ids,
            );
            for input_node in target_nodes {
                if source_node == input_node {
                    continue;
                }

                desired.insert(DesiredNodeLink {
                    output_node: source_node,
                    input_node,
                });
            }
        }
    }

    for (index, combined_node_id) in virtual_output_combined_ids.iter().enumerate() {
        let Some(combined_node_id) = combined_node_id else {
            continue;
        };

        for sink_node_id in resolved_ids_for(resolved, ResolvedGroup::VirtualOutput, index) {
            if sink_node_id == *combined_node_id {
                continue;
            }

            desired.insert(DesiredNodeLink {
                output_node: *combined_node_id,
                input_node: sink_node_id,
            });
        }
    }

    desired.into_iter().collect()
}

fn sync_virtual_input_combined_volumes(
    objects: &Arc<Mutex<PwState>>,
    proxies: &Rc<RefCell<PwProxies>>,
    config: &AppConfig,
    state: &PwState,
) -> Result<()> {
    for index in 0..config.virtual_inputs.len() {
        let Some(combined_node_id) = managed_node_id(state, &virtual_input_combined_name(index))
        else {
            continue;
        };

        let slider = config
            .virtual_inputs
            .get(index)
            .map(|strip| strip.volume)
            .unwrap_or(1.0);
        let desired_linear = human_slider_to_pipewire_linear(slider);

        let should_update = match state.get(&combined_node_id) {
            Some(PwObject::Node(node)) => {
                (node.volume[0] - desired_linear).abs() > 0.01
                    || (node.volume[1] - desired_linear).abs() > 0.01
            }
            _ => false,
        };

        if should_update {
            set_node_volume_impl(objects, proxies, combined_node_id, desired_linear)?;
        }
    }

    Ok(())
}

fn all_nodes_have_known_ports(state: &PwState) -> bool {
    let mut known_port_counts: HashMap<u32, (u32, u32)> = HashMap::new();
    for object in state.values() {
        let PwObject::Port(port) = object else {
            continue;
        };

        let entry = known_port_counts.entry(port.node_id).or_insert((0, 0));
        match port.direction {
            PortDirection::In => entry.0 += 1,
            PortDirection::Out => entry.1 += 1,
        }
    }

    for node in state.values().filter_map(|obj| {
        let PwObject::Node(node) = obj else {
            return None;
        };
        Some(node)
    }) {
        let (known_inputs, known_outputs) =
            known_port_counts.get(&node.id).copied().unwrap_or((0, 0));

        if known_inputs < node.input_ports || known_outputs < node.output_ports {
            return false;
        }
    }

    true
}

fn reconcile_routing_state(state: &BackendState) -> Result<()> {
    if !state.initialized.get() || state.shutdown.get() {
        return Ok(());
    }

    info!("reconciling routing state");

    let Some(config) = state.routing_config.borrow().clone() else {
        return Ok(());
    };

    sync_managed_virtual_devices_impl(
        &state.core,
        &state.registry,
        &state.objects,
        &managed_virtual_strip_names(&config),
    )?;

    let pw_state = state.objects.lock().unwrap().clone();
    let resolved = resolve_nodes_for_config(&config, &pw_state);
    let resolved_ref = ResolvedSetRef {
        physical_inputs: &resolved.physical_inputs,
        virtual_inputs: &resolved.virtual_inputs,
        physical_outputs: &resolved.physical_outputs,
        virtual_outputs: &resolved.virtual_outputs,
    };

    if !all_nodes_have_known_ports(&pw_state) {
        info!("routing reconcile deferred because not all node ports are known yet");
        return Ok(());
    }

    state.meter_manager.borrow_mut().sync_virtual_nodes(
        &state.core,
        &state.objects,
        &meter_target_node_names(&config, resolved_ref, &pw_state),
    )?;

    sync_virtual_input_combined_volumes(&state.objects, &state.proxies, &config, &pw_state)?;

    let desired_links = desired_routing_links(&config, resolved_ref, &pw_state);
    sync_routing_impl(&state.core, &state.registry, &state.objects, &desired_links)?;

    Ok(())
}

// This struct exists because we need to keep the TimerSource alive for the timer
// to not get dropped/deregistered before firing and need to move it into a closure
// without recreating it on every closure call, while also satisfying the lifetime requirement.
#[ouroboros::self_referencing]
struct TimerHandle {
    mainloop: pw::main_loop::MainLoopRc,
    #[borrows(mainloop)]
    #[covariant] // I am praying this is correct
    handle: pw::loop_::TimerSource<'this>,
}

#[derive(Clone)]
struct CmdSender(pw::channel::Sender<BackendCommand>);

impl std::fmt::Debug for CmdSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CmdSender").finish()
    }
}

#[derive(Debug, Clone)]
struct BackendState {
    mainloop: pw::main_loop::MainLoopRc,
    core: pw::core::CoreRc,
    registry: pw::registry::RegistryRc,

    routing_config: Rc<RefCell<Option<AppConfig>>>,
    proxies: Rc<RefCell<PwProxies>>,
    meter_manager: Rc<RefCell<MeterManager>>,
    objects: Arc<Mutex<PwState>>,

    cmd_tx: CmdSender,
    initialized: Rc<Cell<bool>>,
    initial_sync_seq: Rc<RefCell<Option<pw::spa::utils::result::AsyncSeq>>>,

    shutdown: Rc<Cell<bool>>,
    shutdown_sync_seq: Rc<RefCell<Option<pw::spa::utils::result::AsyncSeq>>>,
    shutdown_reply_tx: Rc<RefCell<Option<mpsc::Sender<Result<()>>>>>,
}

impl BackendState {
    fn set_rebuild_timer(&self) {
        self.cmd_tx.0.send(BackendCommand::ResetTimer).unwrap();
    }
}

pub fn pipewire_worker(
    objects: Arc<Mutex<PwState>>,
    meters: Arc<Mutex<HashMap<u32, [f32; 2]>>>,
    cmd_tx: pw::channel::Sender<BackendCommand>,
    cmd_rx: pw::channel::Receiver<BackendCommand>,
    ready_tx: mpsc::Sender<Result<()>>,
) -> JoinHandle<Result<()>> {
    thread::spawn(move || {
        let (mainloop, core) = match create_mainloop() {
            Ok(v) => v,
            Err(err) => {
                let _ = ready_tx.send(Err(err));
                return Ok(());
            }
        };
        let registry = match core.get_registry_rc() {
            Ok(v) => v,
            Err(err) => {
                let _ = ready_tx.send(Err(err.into()));
                return Ok(());
            }
        };

        let state = BackendState {
            mainloop,
            core,
            registry,
            routing_config: Rc::new(RefCell::new(None)),
            proxies: Rc::new(RefCell::new(PwProxies::new())),
            meter_manager: Rc::new(RefCell::new(MeterManager::new(meters))),
            objects,
            cmd_tx: CmdSender(cmd_tx),
            initialized: Rc::new(Cell::new(false)),
            initial_sync_seq: Rc::new(RefCell::new(None)),
            shutdown: Rc::new(Cell::new(false)),
            shutdown_sync_seq: Rc::new(RefCell::new(None)),
            shutdown_reply_tx: Rc::new(RefCell::new(None)),
        };

        let state_done = state.clone();
        let ready_tx_done = ready_tx.clone();

        let _core_listener = state
            .core
            .add_listener_local()
            .info(move |message| {
                info!("{message:?}");
            })
            .error(move |id, seq, res, message| {
                if message.starts_with("enum params") && message.ends_with("failed") {
                    // random errors
                    return;
                }
                error!("{id} seq={seq} res={res}: {message}");
            })
            .done(move |id, seq| {
                if id == pw::core::PW_ID_CORE {
                    if let Some(expected) = *state_done.initial_sync_seq.borrow() {
                        if seq == expected {
                            info!("initial sync complete");
                            state_done.initialized.set(true);
                            let _ = ready_tx_done.send(Ok(()));
                            state_done.set_rebuild_timer();
                        }
                    }

                    if let Some(expected) = *state_done.shutdown_sync_seq.borrow() {
                        if seq == expected {
                            if let Some(reply) = state_done.shutdown_reply_tx.borrow_mut().take() {
                                send_reply(reply, Ok(()));
                            }
                            state_done.mainloop.quit();
                        }
                    }
                }
            })
            .register();

        let state_add = state.clone();
        let state_remove = state.clone();

        let _registry_listener = state
            .registry
            .add_listener_local()
            .global(move |global| {
                let Some(props) = global.props else {
                    return;
                };

                // info!(
                //     "object added: id={} type={} props={:?}",
                //     global.id, global.type_, props
                // );

                let mut objects = state_add.objects.lock().unwrap();
                match global.type_ {
                    ObjectType::Client => {
                        let module_id = props.get(&MODULE_ID).unwrap().parse::<u32>().unwrap();
                        let application_name = props.get(&APP_NAME).unwrap().to_owned();
                        objects.insert(
                            global.id,
                            PwObject::Client(PwClient {
                                module_id,
                                application_name,
                            }),
                        );
                    }
                    ObjectType::Core => {
                        objects.insert(global.id, PwObject::Core);
                    }
                    ObjectType::Device => {
                        handle_device_global(
                            global,
                            props,
                            &mut objects,
                            &state_add.objects,
                            &state_add.registry,
                            &state_add.proxies,
                        );
                    }
                    ObjectType::Factory => {
                        handle_factory_global(global, props, &mut objects);
                    }
                    ObjectType::Metadata => {
                        let name = props.get("metadata.name").unwrap().to_owned();
                        objects.insert(global.id, PwObject::Metadata(name));
                    }
                    ObjectType::Module => {
                        let name = props.get("module.name").unwrap().to_owned();
                        objects.insert(global.id, PwObject::Module(name));
                    }
                    ObjectType::Node => {
                        handle_node_global(
                            global,
                            props,
                            &mut objects,
                            &state_add.objects,
                            &state_add.registry,
                            &state_add.proxies,
                        );
                    }
                    ObjectType::Port => {
                        handle_port_global(
                            global,
                            props,
                            &mut objects,
                            &state_add.objects,
                            &state_add.registry,
                            &state_add.proxies,
                        );
                    }
                    ObjectType::Profiler => {
                        objects.insert(global.id, PwObject::Profiler);
                    }
                    ObjectType::Link => {
                        let client_id = props.get(&CLIENT_ID).unwrap().parse::<u32>().unwrap();
                        let input_node =
                            props.get(&LINK_INPUT_NODE).unwrap().parse::<u32>().unwrap();
                        let input_port =
                            props.get(&LINK_INPUT_PORT).unwrap().parse::<u32>().unwrap();
                        let output_node = props
                            .get(&LINK_OUTPUT_NODE)
                            .unwrap()
                            .parse::<u32>()
                            .unwrap();
                        let output_port = props
                            .get(&LINK_OUTPUT_PORT)
                            .unwrap()
                            .parse::<u32>()
                            .unwrap();

                        objects.insert(
                            global.id,
                            PwObject::Link(PwLink {
                                client_id,
                                input_node,
                                input_port,
                                output_node,
                                output_port,
                            }),
                        );
                    }
                    _ => {
                        warn!("unhandled object type: {}", global.type_);
                    }
                }
                drop(objects);

                if matches!(
                    global.type_,
                    ObjectType::Node | ObjectType::Port | ObjectType::Link
                ) {
                    state_add.set_rebuild_timer();
                }
            })
            .global_remove(move |id| {
                let mut objects = state_remove.objects.lock().unwrap();
                let removed = objects.remove(&id);
                if removed.is_none() {
                    warn!("object removed but not found in state: id={id}");
                } else {
                    if matches!(
                        removed,
                        Some(PwObject::Node(_)) | Some(PwObject::Port(_)) | Some(PwObject::Link(_))
                    ) {
                        state_remove.set_rebuild_timer();
                    }
                }
                state_remove.proxies.borrow_mut().remove(&id);
                drop(objects);
            })
            .register();

        let state_cmd = state.clone();
        let state_timer = state_cmd.clone();
        let timer_handle = TimerHandleBuilder {
            mainloop: state.mainloop.clone(),
            handle_builder: |mainloop| {
                mainloop.loop_().add_timer(move |_| {
                    if let Err(err) = reconcile_routing_state(&state_timer) {
                        error!("failed to reconcile routing state: {err}");
                    }
                })
            },
        }
        .build();

        // This receiver is attached to PipeWire's loop and wakes it through an internal pipe,
        // so frontend commands are processed even when no PipeWire graph events occur.
        let _cmd_source = cmd_rx.attach(state.mainloop.loop_(), move |cmd| match cmd {
            BackendCommand::SetRoutingConfig { config, reply } => {
                *state_cmd.routing_config.as_ref().borrow_mut() = Some(config);

                // We cannot send a message to the channel from teh listener because
                // it will deadlock the channel, so just set the timer directly here.
                timer_handle.with_handle(|handle| {
                    handle.update_timer(Some(Duration::from_millis(10)), None);
                });

                send_reply(reply, Ok(()));
            }
            BackendCommand::SetNodeVolume {
                node_id,
                volume,
                reply,
            } => {
                send_reply(
                    reply,
                    set_node_volume_impl(&state_cmd.objects, &state_cmd.proxies, node_id, volume),
                );
            }
            BackendCommand::Shutdown { reply } => {
                state_cmd.shutdown.set(true);
                *state_cmd.routing_config.as_ref().borrow_mut() = None;
                state_cmd.meter_manager.as_ref().borrow_mut().clear();
                if let Err(err) =
                    remove_managed_virtual_devices_impl(&state_cmd.registry, &state_cmd.objects)
                {
                    error!("failed to cleanup managed virtual devices on shutdown: {err}");
                }

                match state_cmd.core.sync(0) {
                    Ok(seq) => {
                        *state_cmd.shutdown_sync_seq.borrow_mut() = Some(seq);
                        *state_cmd.shutdown_reply_tx.borrow_mut() = Some(reply);
                    }
                    Err(err) => {
                        send_reply(reply, Err(err.into()));
                        state_cmd.mainloop.quit();
                    }
                }
            }
            BackendCommand::ResetTimer => {
                timer_handle.with_handle(|handle| {
                    handle.update_timer(Some(Duration::from_millis(10)), None);
                });
            }
        });

        let initial_sync = match state.core.sync(0) {
            Ok(seq) => seq,
            Err(err) => {
                let _ = ready_tx.send(Err(err.into()));
                return Ok(());
            }
        };
        *state.initial_sync_seq.borrow_mut() = Some(initial_sync);

        state.mainloop.run();
        info!("PipeWire worker thread exiting");

        Ok(())
    })
}
