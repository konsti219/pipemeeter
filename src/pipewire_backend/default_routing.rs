//! Cooperative default-node routing via the `default` metadata.
//!
//! We point WirePlumber's configured default sink and source at our default
//! virtual strips, so freshly-appeared streams land there on their own before
//! our matching logic re-homes them. The configured defaults are persistent
//! session state, so we snapshot what was there first and restore it on exit.

use super::*;

const DEFAULT_SINK_KEY: &str = "default.configured.audio.sink";
const DEFAULT_SOURCE_KEY: &str = "default.configured.audio.source";
const MANAGED_KEYS: [&str; 2] = [DEFAULT_SINK_KEY, DEFAULT_SOURCE_KEY];

const JSON_TYPE: &str = "Spa:String:JSON";

#[derive(Default)]
struct SharedData {
    /// Live value of each managed key, kept current by the listener.
    current: HashMap<String, String>,
    /// Value of each managed key before we first overrode it. An absent key was
    /// unset and is cleared again on restore.
    original: HashMap<String, String>,
    captured: bool,
}

pub struct DefaultRouting {
    metadata_id: Option<u32>,
    proxy: Option<pw::metadata::Metadata>,
    _listener: Option<pw::metadata::MetadataListener>,
    data: Rc<RefCell<SharedData>>,
}

impl std::fmt::Debug for DefaultRouting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefaultRouting")
            .field("attached", &self.proxy.is_some())
            .finish()
    }
}

impl Default for DefaultRouting {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultRouting {
    pub fn new() -> Self {
        Self {
            metadata_id: None,
            proxy: None,
            _listener: None,
            data: Rc::new(RefCell::new(SharedData::default())),
        }
    }

    /// Bind the `default` metadata object. Only the metadata named `default`
    /// should be passed here; ignored if already attached.
    pub fn attach(
        &mut self,
        registry: &pw::registry::RegistryRc,
        global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>,
    ) {
        if self.proxy.is_some() {
            return;
        }

        let proxy = match registry.bind::<pw::metadata::Metadata, _>(global) {
            Ok(proxy) => proxy,
            Err(err) => {
                warn!("failed to bind `default` metadata: {err}");
                return;
            }
        };

        let data = self.data.clone();
        let listener = proxy
            .add_listener_local()
            .property(move |_subject, key, _type, value| {
                if let Some(key) = key.filter(|key| MANAGED_KEYS.contains(key)) {
                    let mut data = data.borrow_mut();
                    match value {
                        Some(value) => data.current.insert(key.to_owned(), value.to_owned()),
                        None => data.current.remove(key),
                    };
                }
                0
            })
            .register();

        self.metadata_id = Some(global.id);
        self.proxy = Some(proxy);
        self._listener = Some(listener);
        info!("attached to `default` metadata for default routing");
    }

    pub fn handle_global_remove(&mut self, id: u32) {
        if self.metadata_id == Some(id) {
            self.metadata_id = None;
            self._listener = None;
            self.proxy = None;
        }
    }

    pub fn ensure_default_sink(&self, node_name: &str) {
        self.set_default(DEFAULT_SINK_KEY, node_name);
    }

    pub fn ensure_default_source(&self, node_name: &str) {
        self.set_default(DEFAULT_SOURCE_KEY, node_name);
    }

    fn set_default(&self, key: &str, node_name: &str) {
        let Some(proxy) = self.proxy.as_ref() else {
            return;
        };

        let desired = format!("{{\"name\":\"{node_name}\"}}");

        {
            let mut data = self.data.borrow_mut();
            if !data.captured {
                data.original = data.current.clone();
                data.captured = true;
                info!("captured pre-existing default routing: {:?}", data.original);
            }
            if data.current.get(key) == Some(&desired) {
                return;
            }
        }

        info!("setting {key} to node '{node_name}'");
        proxy.set_property(0, key, Some(JSON_TYPE), Some(&desired));
        // Reflect the write locally so we don't reissue it before the listener
        // sees the change round-trip back.
        self.data.borrow_mut().current.insert(key.to_owned(), desired);
    }

    /// Restore every managed key to its pre-override value, or clear it if it
    /// was unset, so we don't leave the session pointing at our virtual nodes.
    pub fn restore(&self) {
        let Some(proxy) = self.proxy.as_ref() else {
            return;
        };

        let data = self.data.borrow();
        if !data.captured {
            return;
        }

        for key in MANAGED_KEYS {
            match data.original.get(key) {
                Some(value) => {
                    info!("restoring {key} to '{value}'");
                    proxy.set_property(0, key, Some(JSON_TYPE), Some(value));
                }
                None => {
                    info!("clearing {key}");
                    proxy.set_property(0, key, None, None);
                }
            }
        }
    }
}
