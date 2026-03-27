use std::io::Cursor;

use anyhow::{Context, Result};
use pipewire::{self as pw};
use pw::spa::pod::Pod;

use super::{PwDeviceRoute, PwMediaType};

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

pub(super) fn build_node_volume_props_param(stereo_volume: [f32; 2]) -> Result<Vec<u8>> {
    let object = pw::spa::pod::Object {
        type_: pw::spa::utils::SpaTypes::ObjectParamProps.as_raw(),
        id: pw::spa::param::ParamType::Props.as_raw(),
        properties: vec![pw::spa::pod::Property::new(
            pw::spa::sys::SPA_PROP_channelVolumes,
            pw::spa::pod::Value::ValueArray(pw::spa::pod::ValueArray::Float(vec![
                stereo_volume[0],
                stereo_volume[1],
            ])),
        )],
    };

    let value = pw::spa::pod::Value::Object(object);
    let (cursor, _) =
        pw::spa::pod::serialize::PodSerializer::serialize(Cursor::new(Vec::new()), &value)
            .context("failed to serialize node volume parameter")?;

    Ok(cursor.into_inner())
}

fn pod_u32_value(pod: &Pod) -> Option<u32> {
    if let Ok(value) = pod.get_int() {
        return (value >= 0).then_some(value as u32);
    }
    if let Ok(value) = pod.get_id() {
        return Some(value.0);
    }
    None
}

pub(super) fn route_descriptor_from_param(param: &Pod) -> Option<PwDeviceRoute> {
    let object = param.as_object().ok()?;

    let mut index = None;
    let mut direction = None;
    let mut device = None;
    let mut volume = None;

    for prop in object.props() {
        let key = prop.key().0;
        if key == pw::spa::sys::SPA_PARAM_ROUTE_index {
            index = pod_u32_value(prop.value());
        } else if key == pw::spa::sys::SPA_PARAM_ROUTE_direction {
            direction = pod_u32_value(prop.value());
        } else if key == pw::spa::sys::SPA_PARAM_ROUTE_device {
            device = pod_u32_value(prop.value());
        } else if key == pw::spa::sys::SPA_PARAM_ROUTE_props {
            let Ok(props_object) = prop.value().as_object() else {
                continue;
            };

            for props_prop in props_object.props() {
                if props_prop.key().0 == pw::spa::sys::SPA_PROP_channelVolumes {
                    if let Some(values) = pod_float_array(props_prop.value()) {
                        volume = stereo_from_values(&values);
                    }
                }
            }
        }
    }

    Some(PwDeviceRoute {
        index: index?,
        direction: direction?,
        device: device.unwrap_or(0),
        volume,
    })
}

pub(super) fn build_device_route_volume_param(
    route: PwDeviceRoute,
    stereo_volume: [f32; 2],
) -> Result<Vec<u8>> {
    let props_object = pw::spa::pod::Object {
        type_: pw::spa::utils::SpaTypes::ObjectParamProps.as_raw(),
        id: pw::spa::param::ParamType::Props.as_raw(),
        properties: vec![pw::spa::pod::Property::new(
            pw::spa::sys::SPA_PROP_channelVolumes,
            pw::spa::pod::Value::ValueArray(pw::spa::pod::ValueArray::Float(vec![
                stereo_volume[0],
                stereo_volume[1],
            ])),
        )],
    };

    let route_object = pw::spa::pod::Object {
        type_: pw::spa::utils::SpaTypes::ObjectParamRoute.as_raw(),
        id: pw::spa::param::ParamType::Route.as_raw(),
        properties: vec![
            pw::spa::pod::Property::new(
                pw::spa::sys::SPA_PARAM_ROUTE_index,
                pw::spa::pod::Value::Int(route.index as i32),
            ),
            pw::spa::pod::Property::new(
                pw::spa::sys::SPA_PARAM_ROUTE_direction,
                pw::spa::pod::Value::Id(pw::spa::utils::Id(route.direction)),
            ),
            pw::spa::pod::Property::new(
                pw::spa::sys::SPA_PARAM_ROUTE_device,
                pw::spa::pod::Value::Int(route.device as i32),
            ),
            pw::spa::pod::Property::new(
                pw::spa::sys::SPA_PARAM_ROUTE_props,
                pw::spa::pod::Value::Object(props_object),
            ),
            pw::spa::pod::Property::new(
                pw::spa::sys::SPA_PARAM_ROUTE_save,
                pw::spa::pod::Value::Bool(true),
            ),
        ],
    };

    let value = pw::spa::pod::Value::Object(route_object);
    let (cursor, _) =
        pw::spa::pod::serialize::PodSerializer::serialize(Cursor::new(Vec::new()), &value)
            .context("failed to serialize device route parameter")?;

    Ok(cursor.into_inner())
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
