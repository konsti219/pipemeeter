use std::ffi::CStr;
use std::fmt;

use pipewire::{self as pw};
use pw::spa::pod::Pod;

use super::PwMediaType;

fn stereo_from_values(values: &[f32]) -> Option<[f32; 2]> {
    if values.is_empty() {
        None
    } else if values.len() == 1 {
        Some([values[0], values[0]])
    } else {
        Some([values[0], values[1]])
    }
}

fn pod_float_array(pod: &Pod) -> Option<Vec<f32>> {
    if !pod.is_array() {
        return None;
    }

    let mut n_values = 0u32;
    // Safety: `pod` is a valid reference and libspa writes the number of values to `n_values`.
    let values_ptr =
        unsafe { pw::spa::sys::spa_pod_get_array(pod.as_raw_ptr().cast_const(), &mut n_values) };

    // Safety: The pod is known to be an array pod.
    let array = unsafe { &*(pod.as_raw_ptr().cast_const() as *const pw::spa::sys::spa_pod_array) };

    if n_values == 0 {
        return Some(Vec::new());
    }
    if values_ptr.is_null() {
        return None;
    }
    if array.body.child.type_ != pw::spa::sys::SPA_TYPE_Float {
        return None;
    }
    if array.body.child.size as usize != std::mem::size_of::<f32>() {
        return None;
    }

    // Safety: The array child type/size was validated above and n_values comes from libspa.
    let values = unsafe { std::slice::from_raw_parts(values_ptr as *const f32, n_values as usize) };
    Some(values.to_vec())
}

pub(super) fn node_volume_from_param(param: &Pod) -> Option<[f32; 2]> {
    let object = param.as_object().ok()?;

    let mut channel_volumes: Option<[f32; 2]> = None;
    let mut scalar_volume: Option<f32> = None;

    for prop in object.props() {
        let key = prop.key().0;
        if key == pw::spa::sys::SPA_PROP_channelVolumes {
            if let Some(values) = pod_float_array(prop.value()) {
                if let Some(stereo) = stereo_from_values(&values) {
                    channel_volumes = Some(stereo);
                }
            }
        } else if key == pw::spa::sys::SPA_PROP_volume {
            if let Ok(value) = prop.value().get_float() {
                scalar_volume = Some(value);
            }
        }
    }

    channel_volumes.or_else(|| scalar_volume.map(|value| [value, value]))
}

pub(super) fn media_type_from_enum_format(pod: &Pod) -> PwMediaType {
    let Ok((media_type, media_subtype)) = pw::spa::param::format_utils::parse_format(pod) else {
        return PwMediaType::Unknown;
    };

    use pw::spa::param::format::{MediaSubtype, MediaType};
    if media_type == MediaType::Audio {
        PwMediaType::Audio
    } else if media_type == MediaType::Video || media_type == MediaType::Image {
        PwMediaType::Video
    } else if media_subtype == MediaSubtype::Midi {
        PwMediaType::Midi
    } else {
        PwMediaType::Unknown
    }
}

pub struct DebugPod<'a>(pub &'a Pod);

const POD_DEBUG_MAX_DEPTH: usize = 6;

struct DebugPodWithDepth<'a> {
    pod: &'a Pod,
    depth: usize,
}

struct DebugPodPropWithDepth<'a> {
    prop: &'a pw::spa::pod::PodProp,
    depth: usize,
}

impl fmt::Debug for DebugPod<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        DebugPodWithDepth {
            pod: self.0,
            depth: 0,
        }
        .fmt(f)
    }
}

impl fmt::Debug for DebugPodWithDepth<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let pod = self.pod;

        if pod.is_none() {
            return f.debug_tuple("Pod").field(&()).finish();
        } else if pod.is_bool() {
            return debug_scalar_result(f, pod.get_bool());
        } else if pod.is_id() {
            return debug_scalar_result(f, pod.get_id());
        } else if pod.is_int() {
            return debug_scalar_result(f, pod.get_int());
        } else if pod.is_long() {
            return debug_scalar_result(f, pod.get_long());
        } else if pod.is_float() {
            return debug_scalar_result(f, pod.get_float());
        } else if pod.is_double() {
            return debug_scalar_result(f, pod.get_double());
        } else if pod.is_fd() {
            return debug_scalar_result(f, pod.get_fd());
        } else if pod.is_rectangle() {
            return debug_scalar_result(f, pod.get_rectangle());
        } else if pod.is_fraction() {
            return debug_scalar_result(f, pod.get_fraction());
        } else if pod.is_pointer() {
            return debug_scalar_result(f, pod.get_pointer());
        } else if pod.is_bytes() {
            let value = pod
                .get_bytes()
                .map(summarize_bytes)
                .map_err(|err| err.to_string());
            return debug_scalar_result(f, value);
        } else if pod.is_string() {
            return debug_scalar_result(f, pod_string(pod));
        }

        let mut out = f.debug_struct("Pod");
        out.field("type", &pod.type_());

        if self.depth >= POD_DEBUG_MAX_DEPTH {
            out.field("truncated", &true);
            return out.finish();
        }

        if pod.is_array() {
            out.field("kind", &"Array");
            out.field("values", &pod_array_values(pod, self.depth + 1));
        } else if pod.is_choice() {
            out.field("kind", &"Choice");
        } else if pod.is_bitmap() {
            out.field("kind", &"Bitmap");
        } else if pod.is_struct() {
            match pod.as_struct() {
                Ok(struct_pod) => {
                    let fields = struct_pod
                        .fields()
                        .map(|field| DebugPodWithDepth {
                            pod: field,
                            depth: self.depth + 1,
                        })
                        .collect::<Vec<_>>();
                    out.field("fields", &fields);
                }
                Err(err) => {
                    out.field("struct_error", &err);
                }
            }
        } else if pod.is_object() {
            match pod.as_object() {
                Ok(object_pod) => {
                    let props = object_pod
                        .props()
                        .map(|prop| DebugPodPropWithDepth {
                            prop,
                            depth: self.depth + 1,
                        })
                        .collect::<Vec<_>>();
                    out.field("object_type", &object_pod.type_());
                    out.field("object_id", &object_pod.id());
                    out.field("props", &props);
                }
                Err(err) => {
                    out.field("object_error", &err);
                }
            }
        } else if pod.is_sequence() {
            out.field("kind", &"Sequence");
        }

        out.finish()
    }
}

fn debug_scalar_result<T, E>(f: &mut fmt::Formatter<'_>, value: Result<T, E>) -> fmt::Result
where
    T: fmt::Debug,
    E: fmt::Display,
{
    match value {
        Ok(value) => f.debug_tuple("Pod").field(&value).finish(),
        Err(err) => f
            .debug_tuple("Pod")
            .field(&format!("<decode-error: {err}>"))
            .finish(),
    }
}

impl fmt::Debug for DebugPodPropWithDepth<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = f.debug_struct("Prop");
        out.field("key", &self.prop.key());
        out.field(
            "value",
            &DebugPodWithDepth {
                pod: self.prop.value(),
                depth: self.depth,
            },
        );
        out.finish()
    }
}

fn summarize_bytes(bytes: &[u8]) -> String {
    let preview_len = bytes.len().min(24);
    let preview = bytes
        .iter()
        .take(preview_len)
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ");

    if bytes.len() > preview_len {
        format!("len={} [{} ...]", bytes.len(), preview)
    } else {
        format!("len={} [{}]", bytes.len(), preview)
    }
}

fn pod_string(pod: &Pod) -> Result<String, String> {
    let mut raw = std::ptr::null();
    // Safety: libspa fills `raw` with a valid NUL-terminated pointer as long as
    // `pod` is a valid string pod and lives for the duration of this call.
    let res = unsafe { pw::spa::sys::spa_pod_get_string(pod.as_raw_ptr().cast_const(), &mut raw) };
    if res < 0 {
        return Err(format!("spa_pod_get_string failed: {}", -res));
    }
    if raw.is_null() {
        return Err("spa_pod_get_string returned null".to_owned());
    }

    // Safety: `raw` is expected to point to a valid C string according to libspa contract.
    let c_str = unsafe { CStr::from_ptr(raw) };
    c_str
        .to_str()
        .map(ToOwned::to_owned)
        .map_err(|err| format!("invalid UTF-8 string pod: {err}"))
}

fn pod_array_values(pod: &Pod, depth: usize) -> Result<Vec<String>, String> {
    assert!(pod.is_array(),);

    let mut n_values = 0u32;
    // Safety: `pod` is a valid pod reference and `n_values` points to writable memory.
    let values_ptr =
        unsafe { pw::spa::sys::spa_pod_get_array(pod.as_raw_ptr().cast_const(), &mut n_values) };

    // Safety: `pod` is known to be an array pod at callsites.
    let array = unsafe { &*(pod.as_raw_ptr().cast_const() as *const pw::spa::sys::spa_pod_array) };
    let child = array.body.child;
    let child_size: usize = child.size as usize;

    if n_values > 0 && values_ptr.is_null() {
        return Err("spa_pod_get_array returned null values pointer".to_owned());
    }
    if n_values > 0 && child_size == 0 {
        return Err("array child size is zero".to_owned());
    }

    let elem_padding = (8 - (child_size % 8)) % 8;

    let mut out = Vec::with_capacity(n_values as usize);
    for i in 0..n_values as usize {
        // Safety: values_ptr points to n_values contiguous entries of child_size bytes.
        let value_ptr = unsafe { (values_ptr as *const u8).add(i * child_size) };
        // Safety: value_ptr points to child_size readable bytes for this element.
        let value_bytes = unsafe { std::slice::from_raw_parts(value_ptr, child_size) };

        let mut elem_pod = Vec::with_capacity(
            std::mem::size_of::<pw::spa::sys::spa_pod>() + child_size + elem_padding,
        );
        elem_pod.extend_from_slice(&child.size.to_ne_bytes());
        elem_pod.extend_from_slice(&child.type_.to_ne_bytes());
        elem_pod.extend_from_slice(value_bytes);
        elem_pod.resize(elem_pod.len() + elem_padding, 0);

        let Some(elem) = Pod::from_bytes(&elem_pod) else {
            return Err(format!("failed to parse array element at index {i}"));
        };

        out.push(format!("{:?}", DebugPodWithDepth { pod: elem, depth }));
    }

    Ok(out)
}
