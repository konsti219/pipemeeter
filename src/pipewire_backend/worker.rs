use super::*;

pub fn pipewire_worker(
    objects: Arc<Mutex<PwState>>,
    meters: Arc<Mutex<HashMap<u32, [f32; 2]>>>,
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

        let proxies = Rc::new(RefCell::new(PwProxies::new()));

        let _core_listener = core
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
                info!("worker done: id={id} seq={seq:?}");
            })
            .register();

        let objects_add = objects.clone();
        let proxies_add = proxies.clone();
        let registry_add = registry.clone();
        let objects_remove = objects.clone();
        let proxies_remove = proxies.clone();

        let _registry_listener = registry
            .add_listener_local()
            .global(move |global| {
                let Some(props) = global.props else {
                    return;
                };

                // info!(
                //     "object added: id={} type={} props={:?}",
                //     global.id, global.type_, props
                // );

                let mut objects = objects_add.lock().unwrap();
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
                            &objects_add,
                            &registry_add,
                            &proxies_add,
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
                            &objects_add,
                            &registry_add,
                            &proxies_add,
                        );
                    }
                    ObjectType::Port => {
                        handle_port_global(
                            global,
                            props,
                            &mut objects,
                            &objects_add,
                            &registry_add,
                            &proxies_add,
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
                        let managed_by_pipemeeter = props.get("pipemeeter.managed").is_some();

                        objects.insert(
                            global.id,
                            PwObject::Link(PwLink {
                                client_id,
                                input_node,
                                input_port,
                                output_node,
                                output_port,
                                managed_by_pipemeeter,
                            }),
                        );
                    }
                    _ => {
                        warn!("unhandled object type: {}", global.type_);
                    }
                }
            })
            .global_remove(move |id| {
                let mut objects = objects_remove.lock().unwrap();
                if let Some(_object) = objects.remove(&id) {
                    // info!("object removed: id={} object={:?}", id, _object);
                } else {
                    warn!("object removed but not found in state: id={id}");
                }
                proxies_remove.borrow_mut().remove(&id);
            })
            .register();

        let cmd_mainloop = mainloop.clone();
        let meter_manager = RefCell::new(MeterManager::new(meters));

        // This receiver is attached to PipeWire's loop and wakes it through an internal pipe,
        // so frontend commands are processed even when no PipeWire graph events occur.
        let _cmd_source = cmd_rx.attach(mainloop.loop_(), move |cmd| match cmd {
            BackendCommand::SyncManagedVirtualDevices { names, reply } => {
                send_reply(
                    reply,
                    sync_managed_virtual_devices_impl(&core, &registry, &objects, &names),
                );
            }
            BackendCommand::SetNodeVolume {
                node_id,
                volume,
                reply,
            } => {
                send_reply(
                    reply,
                    set_node_volume_impl(&objects, &proxies, node_id, volume),
                );
            }
            BackendCommand::SyncRouting { links, reply } => {
                send_reply(reply, sync_routing_impl(&core, &registry, &objects, &links));
            }
            BackendCommand::SyncVirtualMeters { names, reply } => {
                send_reply(
                    reply,
                    meter_manager
                        .borrow_mut()
                        .sync_virtual_nodes(&core, &objects, &names),
                );
            }
            BackendCommand::Shutdown { reply } => {
                meter_manager.borrow_mut().clear();
                if let Err(err) = remove_managed_links_impl(&registry, &objects) {
                    error!("failed to cleanup managed links on shutdown: {err}");
                }
                if let Err(err) = remove_managed_virtual_devices_impl(&registry, &objects) {
                    error!("failed to cleanup managed virtual devices on shutdown: {err}");
                }
                send_reply(reply, Ok(()));
                cmd_mainloop.quit();
            }
        });

        let _ = ready_tx.send(Ok(()));
        mainloop.run();
        info!("PipeWire worker thread exiting");

        Ok(())
    })
}
