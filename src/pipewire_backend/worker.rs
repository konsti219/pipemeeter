use super::*;
use crate::config::AppConfig;
use crate::volume::slider_to_pipewire_linear;
use std::cell::Cell;
use std::collections::HashSet;

fn meter_target_node_names(config: &AppConfig, state: &PwState) -> Vec<String> {
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

    for strip in &config.physical_inputs {
        for node_id in &strip.resolved_nodes {
            if let Some(name) = id_to_name.get(node_id) {
                names.insert(name.clone());
            }
        }
    }

    for strip in &config.physical_outputs {
        for node_id in &strip.resolved_nodes {
            if let Some(name) = id_to_name.get(node_id) {
                names.insert(name.clone());
            }
        }
    }

    names.into_iter().collect()
}

fn output_target_nodes_for_route(
    config: &AppConfig,
    route_index: usize,
    virtual_output_combined_ids: &[Option<u32>],
) -> Vec<u32> {
    if route_index < config.physical_outputs.len() {
        config
            .physical_outputs
            .get(route_index)
            .map(|strip| strip.resolved_nodes.clone())
            .unwrap_or_default()
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

fn desired_routing_links(config: &AppConfig, state: &PwState) -> Vec<DesiredNodeLink> {
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

        for source_node_id in config
            .virtual_inputs
            .get(index)
            .map(|strip| strip.resolved_nodes.as_slice())
            .unwrap_or_default()
        {
            desired.insert(DesiredNodeLink {
                output_node: *source_node_id,
                input_node: *combined_node_id,
            });
        }
    }

    for strip in &config.physical_inputs {
        let source_nodes = strip.resolved_nodes.as_slice();

        for (route_index, enabled) in strip.routes_to_outputs.iter().copied().enumerate() {
            if !enabled {
                continue;
            }

            let target_nodes =
                output_target_nodes_for_route(config, route_index, &virtual_output_combined_ids);

            for output_node in source_nodes {
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

            let target_nodes =
                output_target_nodes_for_route(config, route_index, &virtual_output_combined_ids);
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

        for sink_node_id in config
            .virtual_outputs
            .get(index)
            .map(|strip| strip.resolved_nodes.as_slice())
            .unwrap_or_default()
        {
            if *sink_node_id == *combined_node_id {
                continue;
            }

            desired.insert(DesiredNodeLink {
                output_node: *combined_node_id,
                input_node: *sink_node_id,
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
        let desired_linear = slider_to_pipewire_linear(slider);

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

    // Always lock config before objects to avoid lock-order inversion.
    let mut config = state.config.lock().unwrap();
    let pw_state = state.objects.lock().unwrap().clone();

    resolve_nodes(&mut config, &pw_state);

    sync_managed_virtual_devices_impl(
        &state.core,
        &state.registry,
        &state.objects,
        &managed_virtual_strip_names(&config),
    )?;

    if !all_nodes_have_known_ports(&pw_state) {
        info!("routing reconcile deferred because not all node ports are known yet");
        return Ok(());
    }

    state.meter_manager.borrow_mut().sync_virtual_nodes(
        &state.core,
        &state.objects,
        &meter_target_node_names(&config, &pw_state),
    )?;

    sync_virtual_input_combined_volumes(&state.objects, &state.proxies, &config, &pw_state)?;

    let desired_links = desired_routing_links(&config, &pw_state);
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

    config: Arc<Mutex<AppConfig>>,
    objects: Arc<Mutex<PwState>>,
    proxies: Rc<RefCell<PwProxies>>,
    meter_manager: Rc<RefCell<MeterManager>>,

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
    config: Arc<Mutex<AppConfig>>,
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
            config,
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
            BackendCommand::UpdateRouting => {
                // We cannot send a message to the channel from teh listener because
                // it will deadlock the channel, so just set the timer directly here.
                timer_handle.with_handle(|handle| {
                    handle.update_timer(Some(Duration::from_millis(10)), None);
                });
            }
            BackendCommand::SetNodeVolume { node_id, volume } => {
                if let Err(err) =
                    set_node_volume_impl(&state_cmd.objects, &state_cmd.proxies, node_id, volume)
                {
                    error!("failed to set node volume: {err}");
                }
            }
            BackendCommand::Shutdown { reply } => {
                state_cmd.shutdown.set(true);
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
