use std::collections::{HashMap, HashSet};
use std::mem;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use log::warn;
use pipewire as pw;
use pw::properties::properties;
use pw::spa;

use super::{PwObject, PwState};

pub const METER_STREAM_NODE_PREFIX: &str = "pipemeeter/meter-";

struct MeterUserData {
    format: spa::param::audio::AudioInfoRaw,
}

struct MeterTap {
    _stream: pw::stream::StreamRc,
    _listener: pw::stream::StreamListener<MeterUserData>,
}

impl std::fmt::Debug for MeterTap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeterTap").finish()
    }
}

fn linear_peak_to_meter_level(peak: f32) -> f32 {
    peak.max(0.0).cbrt()
}

#[derive(Debug)]
pub struct MeterManager {
    taps: HashMap<u32, MeterTap>,
    meters: Arc<Mutex<HashMap<u32, [f32; 2]>>>,
}

impl MeterManager {
    pub fn new(meters: Arc<Mutex<HashMap<u32, [f32; 2]>>>) -> Self {
        Self {
            meters,
            taps: HashMap::new(),
        }
    }

    pub fn clear(&mut self) {
        self.taps.clear();
        self.meters.lock().unwrap().clear();
    }

    pub fn sync_virtual_nodes(
        &mut self,
        core: &pw::core::CoreRc,
        objects: &Arc<Mutex<PwState>>,
        desired_node_names: &[String],
    ) -> Result<()> {
        let desired_nodes = {
            let objects = objects.lock().unwrap();
            let desired_name_set = desired_node_names
                .iter()
                .map(String::as_str)
                .collect::<HashSet<_>>();

            objects
                .values()
                .filter_map(|obj| {
                    let PwObject::Node(node) = obj else {
                        return None;
                    };

                    if desired_name_set.contains(node.name.as_str()) {
                        Some((node.id, node.name.clone()))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };

        let desired_ids = desired_nodes
            .iter()
            .map(|(id, _)| *id)
            .collect::<HashSet<_>>();

        let removed_ids = self
            .taps
            .keys()
            .copied()
            .filter(|id| !desired_ids.contains(id))
            .collect::<Vec<_>>();
        for id in removed_ids {
            self.taps.remove(&id);
            self.meters.lock().unwrap().remove(&id);
        }

        for (node_id, _node_name) in desired_nodes {
            if self.taps.contains_key(&node_id) {
                continue;
            }

            match create_meter_tap(core, node_id, self.meters.clone()) {
                Ok(tap) => {
                    self.taps.insert(node_id, tap);
                }
                Err(err) => {
                    warn!("failed to create meter tap for node #{}: {}", node_id, err);
                }
            }
        }

        Ok(())
    }
}

fn create_meter_tap(
    core: &pw::core::CoreRc,
    node_id: u32,
    meters: Arc<Mutex<HashMap<u32, [f32; 2]>>>,
) -> Result<MeterTap> {
    let props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "DSP",
        *pw::keys::NODE_NAME => format!("{}{node_id}", METER_STREAM_NODE_PREFIX),
        *pw::keys::STREAM_MONITOR => "true",
    };

    let stream = pw::stream::StreamRc::new(
        core.clone(),
        &format!("pipemeeter-meter-{}", node_id),
        props,
    )?;

    let peak_store = meters.clone();
    let listener = stream
        .add_local_listener_with_user_data(MeterUserData {
            format: Default::default(),
        })
        .param_changed(move |_stream, user_data, id, param| {
            let Some(param) = param else {
                return;
            };
            if id != pw::spa::param::ParamType::Format.as_raw() {
                return;
            }

            let Ok((media_type, media_subtype)) = spa::param::format_utils::parse_format(param)
            else {
                return;
            };

            if media_type != spa::param::format::MediaType::Audio
                || media_subtype != spa::param::format::MediaSubtype::Raw
            {
                return;
            }

            let _ = user_data.format.parse(param);
        })
        .process(move |stream, user_data| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buffer.datas_mut();
            if datas.is_empty() {
                return;
            }

            let data = &mut datas[0];
            let n_samples = data.chunk().size() as usize / mem::size_of::<f32>();
            if n_samples == 0 {
                return;
            }

            let Some(samples) = data.data() else {
                return;
            };

            let n_channels = user_data.format.channels().max(1) as usize;
            let frame_count = n_samples / n_channels;
            if frame_count == 0 {
                return;
            }

            let mut peaks = [0.0_f32, 0.0_f32];

            for frame in 0..frame_count {
                for channel in 0..2 {
                    let channel_idx = channel.min(n_channels - 1);
                    let sample_idx = frame * n_channels + channel_idx;
                    let byte_idx = sample_idx * mem::size_of::<f32>();
                    if byte_idx + mem::size_of::<f32>() > samples.len() {
                        continue;
                    }

                    let bytes = [
                        samples[byte_idx],
                        samples[byte_idx + 1],
                        samples[byte_idx + 2],
                        samples[byte_idx + 3],
                    ];
                    let value = f32::from_le_bytes(bytes).abs();
                    if value > peaks[channel] {
                        peaks[channel] = value;
                    }
                }
            }

            peak_store.lock().unwrap().insert(
                node_id,
                [
                    linear_peak_to_meter_level(peaks[0]),
                    linear_peak_to_meter_level(peaks[1]),
                ],
            );
        })
        .register()?;

    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    let obj = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };
    let param_bytes: Vec<u8> = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(obj),
    )
    .unwrap()
    .0
    .into_inner();

    let param = spa::pod::Pod::from_bytes(&param_bytes).unwrap();
    let mut params = [param];

    stream.connect(
        spa::utils::Direction::Input,
        None,
        pw::stream::StreamFlags::MAP_BUFFERS | pw::stream::StreamFlags::RT_PROCESS,
        &mut params,
    )?;

    Ok(MeterTap {
        _stream: stream,
        _listener: listener,
    })
}
