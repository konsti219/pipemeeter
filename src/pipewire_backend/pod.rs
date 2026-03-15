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

    for prop in object.props() {
        let key = prop.key().0;
        if key == pw::spa::sys::SPA_PROP_channelVolumes {
            if let Some(values) = pod_float_array(prop.value()) {
                if let Some(stereo) = stereo_from_values(&values) {
                    return Some(stereo);
                }
            }
        }
    }

    None
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
