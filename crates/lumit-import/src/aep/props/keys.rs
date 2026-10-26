//! Keyframes: the `lhd3` header and the `ldat` records under a property, and
//! the eases After Effects works out from them.

use super::*;

/// The `lhd3` header and the `ldat` records it describes, as an iterator over
/// borrowed record slices. `None` when there is no keyframe list at all.
pub(super) fn records(
    inside: &[Chunk<'_>],
) -> Option<(usize, usize, impl Iterator<Item = Vec<u8>>)> {
    let list = inside.iter().find(|chunk| chunk.is_list(b"list"))?;
    let inner: Vec<Chunk<'_>> = list.children().ok().collect();
    let header = inner.iter().find(|chunk| chunk.id == *b"lhd3")?;
    let count = usize::from(u16_at(header.body, lhd3::COUNT)?);
    let size = usize::from(u16_at(header.body, lhd3::ITEM_SIZE)?);
    let kind = usize::from(u8_at(header.body, lhd3::TYPE)?);
    let data: Vec<u8> = inner
        .iter()
        .find(|chunk| chunk.id == *b"ldat")
        .map_or_else(Vec::new, |chunk| chunk.body.to_vec());
    // The count and the size both come out of the file, so the product is
    // checked against the bytes that are actually there rather than trusted.
    let count = count.min(data.len().checked_div(size).unwrap_or_default());
    Some((
        kind,
        size,
        (0..count).map(move |index| {
            data.get(index * size..(index + 1) * size)
                .unwrap_or_default()
                .to_vec()
        }),
    ))
}

/// A keyframe's time, in comp seconds. The file counts internal timebase units
/// from the layer's start; scripting reports seconds from the comp's.
pub(super) fn time_of(record: &[u8], ctx: Ctx<'_>) -> f64 {
    let units = f64::from(i32_at(record, 0).unwrap_or_default());
    let base = if ctx.timebase == 0.0 {
        1.0
    } else {
        ctx.timebase
    };
    ctx.start + units / base
}

/// Every keyframe of one property.
///
/// The record size is read from `lhd3` and paired with the list's type code to
/// choose the layout — never assumed, because the size *is* the class. A pair
/// no table knows is an error rather than a guess, and the caller falls the
/// property back to its static value with the class in the note.
pub(super) fn keyframes(
    inside: &[Chunk<'_>],
    dimensions: usize,
    colour: bool,
    spatial: bool,
    scale: &[f64],
    ctx: Ctx<'_>,
) -> Result<Vec<Keyframe>, String> {
    let Some((kind, size, items)) = records(inside) else {
        return Err("the property is animated but carries no keyframe list".to_string());
    };

    let layout = match (kind, size) {
        (4, 152) => Layout::Colour,
        (4, 128) if spatial => Layout::Spatial(3),
        (4, 128) => Layout::Plain(3),
        (4, 104) => Layout::Spatial(2),
        (4, 88) => Layout::Plain(2),
        (4, 80) => Layout::Plain(1),
        (4, 64) => Layout::Valueless,
        (4, 48) => Layout::Plain(1),
        _ => {
            return Err(format!(
                "keyframe class {kind}/{size} is not decoded yet; the static value is used instead"
            ))
        }
    };

    Ok(items
        .map(|record| {
            let mut key = Keyframe {
                t: Some(time_of(&record, ctx)),
                in_interp: Some(enums::interpolation(u8_at(&record, 4).unwrap_or_default())),
                out_interp: Some(enums::interpolation(u8_at(&record, 5).unwrap_or_default())),
                ..Keyframe::default()
            };
            let temporal = u8_at(&record, 7).unwrap_or_default();
            key.roving = Some(bit(temporal, 5));
            key.auto_bezier = Some(bit(temporal, 4));
            key.continuous = Some(bit(temporal, 3));

            let payload = record.get(8..).unwrap_or_default();
            match layout {
                // Both of these begin with two doubles this reader does not
                // claim to understand, so the ease starts one double in.
                Layout::Colour => {
                    let values = doubles(payload);
                    ease_into(&mut key, values.get(1..).unwrap_or_default(), 1, &[]);
                    key.v = values
                        .get(6..10)
                        .and_then(|rgba| resolve(rgba, 4, true, &[]));
                }
                Layout::Valueless => {
                    let values = doubles(payload);
                    ease_into(&mut key, values.get(1..).unwrap_or_default(), 1, &[]);
                }
                Layout::Spatial(axes) => {
                    let values = doubles(payload.get(8..).unwrap_or_default());
                    // One ease for the whole property, which is what the DOM
                    // returns for a spatial one.
                    ease_into(&mut key, &values, 1, &[]);
                    let take = |from: usize| -> Option<Vec<f64>> {
                        Some(values.get(5 + from * axes..5 + (from + 1) * axes)?.to_vec())
                    };
                    key.v = take(0).and_then(|v| resolve(&v, axes, false, scale));
                    key.in_tangent = take(1);
                    key.out_tangent = take(2);
                    let flags = u8_at(payload, 3).unwrap_or_default();
                    key.spatial_auto_bezier = Some(bit(flags, 1));
                    key.spatial_continuous = Some(bit(flags, 0));
                }
                Layout::Plain(axes) => {
                    let values = doubles(payload);
                    key.v = values
                        .get(..axes)
                        .and_then(|v| resolve(v, dimensions.min(axes), colour, scale));
                    ease_into(&mut key, &values, axes, scale);
                }
            }
            key
        })
        .collect())
}

/// After Effects' default keyframe influence: a sixth of the segment.
pub(super) const DEFAULT_INFLUENCE: f64 = 100.0 / 6.0;

/// Finish the ease the file only half-stores.
///
/// This is the one place the parser *computes* rather than reads, and it has to
/// be: After Effects writes zeros into the ease of a linear or held key and
/// works the numbers out when asked, so a straight copy of the bytes gives
/// every linear key a speed of nought — a curve that stands still where the
/// real one moves. The rule is the DOM's own, in three cases:
///
/// - **held**: no speed, the default influence;
/// - **linear**: the constant speed of the segment it joins — the slope
///   between this key and its neighbour, or nought at the ends and against a
///   held neighbour — with the default influence;
/// - **bezier**: the stored numbers, except at the two ends, where the outward
///   side has no segment and so no speed.
///
/// A spatial property's speed is the *length* of the velocity, one number for
/// the whole property; every other multi-dimensional one gets a number per
/// axis.
pub(super) fn resolve_ease(keys: &mut [Keyframe], spatial: bool) {
    let times: Vec<f64> = keys.iter().map(|key| key.t.unwrap_or_default()).collect();
    let values: Vec<Vec<f64>> = keys.iter().map(|key| axes_of(key.v.as_ref())).collect();
    let interps: Vec<(String, String)> = keys
        .iter()
        .map(|key| {
            (
                key.in_interp.clone().unwrap_or_default(),
                key.out_interp.clone().unwrap_or_default(),
            )
        })
        .collect();

    for index in 0..keys.len() {
        for outgoing in [false, true] {
            let (mine, theirs) = if outgoing {
                (interps[index].1.as_str(), index.checked_add(1))
            } else {
                (interps[index].0.as_str(), index.checked_sub(1))
            };
            let neighbour = theirs.filter(|other| *other < keys.len());
            let width = |key: &Keyframe| {
                let side = if outgoing {
                    &key.out_ease
                } else {
                    &key.in_ease
                };
                side.as_deref().map_or(1, <[Ease]>::len).max(1)
            };
            let default = |speeds: Vec<f64>, count: usize| -> Vec<Ease> {
                (0..count.max(speeds.len()))
                    .map(|axis| Ease {
                        speed: Some(speeds.get(axis).copied().unwrap_or_default()),
                        influence: Some(DEFAULT_INFLUENCE),
                    })
                    .collect()
            };

            let replacement = match mine {
                "HOLD" => Some(default(Vec::new(), width(&keys[index]))),
                "LINEAR" => {
                    let held = neighbour.is_some_and(|other| {
                        let side = if outgoing {
                            &interps[other].0
                        } else {
                            &interps[other].1
                        };
                        side == "HOLD"
                    });
                    let speeds = match neighbour.filter(|_| !held) {
                        Some(other) => {
                            let (a, b) = if outgoing {
                                (index, other)
                            } else {
                                (other, index)
                            };
                            let axes = |at: usize| values.get(at).map(Vec::as_slice).unwrap_or(&[]);
                            let tangent = |at: usize, out: bool| {
                                let side = if out {
                                    &keys[at].out_tangent
                                } else {
                                    &keys[at].in_tangent
                                };
                                side.as_deref().unwrap_or_default()
                            };
                            segment_speed(
                                times[b] - times[a],
                                (axes(a), tangent(a, true)),
                                (axes(b), tangent(b, false)),
                                spatial,
                            )
                        }
                        None => Vec::new(),
                    };
                    Some(default(speeds, width(&keys[index])))
                }
                // A bezier key at either end of the run has no segment on its
                // outward side, so the stored speed there is not a speed.
                "BEZIER" if neighbour.is_none() => {
                    let side = if outgoing {
                        &keys[index].out_ease
                    } else {
                        &keys[index].in_ease
                    };
                    Some(
                        side.as_deref()
                            .unwrap_or_default()
                            .iter()
                            .map(|ease| Ease {
                                speed: Some(0.0),
                                influence: ease.influence,
                            })
                            .collect(),
                    )
                }
                _ => None,
            };

            if let Some(replacement) = replacement {
                if outgoing {
                    keys[index].out_ease = Some(replacement);
                } else {
                    keys[index].in_ease = Some(replacement);
                }
            }
        }
    }
}

/// The constant speed across one segment: per axis, or — for a spatial
/// property — the length of the path travelled, once, for the whole property.
///
/// The spatial case is the interesting one. After Effects does not measure the
/// straight line between two keys, it measures the **motion path**: the cubic
/// through the two keys' own spatial handles, which bows away from the straight
/// line whenever the handles are not flat. Measuring the chord instead reports a
/// layer as moving slower than it does — on the golden fixture, by up to 2.5%.
/// The curve is walked rather than solved, which is exact enough for a number
/// the DOM itself prints to fourteen figures and cheap enough for the handful of
/// keys a property has.
pub(super) fn segment_speed(
    seconds: f64,
    from: (&[f64], &[f64]),
    to: (&[f64], &[f64]),
    spatial: bool,
) -> Vec<f64> {
    let (from, from_handle) = from;
    let (to, to_handle) = to;
    if seconds == 0.0 || from.is_empty() || from.len() != to.len() {
        return Vec::new();
    }
    if from.len() == 1 {
        let a = from.first().copied().unwrap_or_default();
        let b = to.first().copied().unwrap_or_default();
        return vec![(b - a) / seconds];
    }
    if spatial {
        return vec![path_length(from, from_handle, to, to_handle) / seconds];
    }
    from.iter()
        .zip(to)
        .map(|(a, b)| (b - a) / seconds)
        .collect()
}

/// How many straight pieces the motion path is measured in. Enough that the
/// answer agrees with After Effects' own to six significant figures on the
/// golden fixture, and fixed so the number is the same on every machine.
const PATH_STEPS: usize = 1024;

/// The length of the cubic between two keys, through their spatial handles. A
/// missing or mismatched handle falls back to the straight line, which is what
/// the curve becomes when both handles are flat anyway.
pub(super) fn path_length(from: &[f64], from_handle: &[f64], to: &[f64], to_handle: &[f64]) -> f64 {
    let axes = from.len();
    let handles = from_handle.len() == axes && to_handle.len() == axes;
    let control = |base: &[f64], handle: &[f64], axis: usize| {
        base.get(axis).copied().unwrap_or_default()
            + if handles {
                handle.get(axis).copied().unwrap_or_default()
            } else {
                0.0
            }
    };
    let at = |t: f64, axis: usize| {
        let u = 1.0 - t;
        let p0 = from.get(axis).copied().unwrap_or_default();
        let p3 = to.get(axis).copied().unwrap_or_default();
        let p1 = control(from, from_handle, axis);
        let p2 = control(to, to_handle, axis);
        u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3
    };

    let mut length = 0.0;
    let mut previous: Vec<f64> = (0..axes).map(|axis| at(0.0, axis)).collect();
    for step in 1..=PATH_STEPS {
        #[allow(clippy::cast_precision_loss)]
        let t = step as f64 / PATH_STEPS as f64;
        let point: Vec<f64> = (0..axes).map(|axis| at(t, axis)).collect();
        length += previous
            .iter()
            .zip(&point)
            .map(|(a, b)| (b - a) * (b - a))
            .sum::<f64>()
            .sqrt();
        previous = point;
    }
    length
}

/// A keyframe value read as a run of numbers — one for a scalar, several for a
/// vector, and none for a value that is not a number at all (a path).
pub(super) fn axes_of(value: Option<&serde_json::Value>) -> Vec<f64> {
    match value {
        Some(serde_json::Value::Array(items)) => {
            items.iter().filter_map(serde_json::Value::as_f64).collect()
        }
        Some(other) => other.as_f64().into_iter().collect(),
        None => Vec::new(),
    }
}

/// Which shape a keyframe record takes, once its size has said so.
#[derive(Debug, Clone, Copy)]
enum Layout {
    /// R, G, B, A after the ease.
    Colour,
    /// Position and its friends: one ease for the property, then the value and
    /// two spatial tangents, `axes` wide each.
    Spatial(usize),
    /// The ordinary case: `axes` values, then `axes` of each ease number.
    Plain(usize),
    /// A key with no value of its own — a mask path's, whose value lives
    /// elsewhere, or a marker's.
    Valueless,
}

/// Fill in a key's temporal ease from a run of doubles laid out as
/// `value…, in speed…, in influence…, out speed…, out influence…`.
///
/// Influence is a fraction on disk and a percentage in the DOM. Speed is in the
/// property's own units per second on both sides, so it carries the same
/// multiplier the value does — a percent property's speed is percent per
/// second.
pub(super) fn ease_into(key: &mut Keyframe, values: &[f64], axes: usize, scale: &[f64]) {
    let side = |from: usize| -> Vec<Ease> {
        (0..axes)
            .map(|axis| {
                let speed = values.get(axes * from + axis).copied().unwrap_or_default();
                let influence = values
                    .get(axes * (from + 1) + axis)
                    .copied()
                    .unwrap_or_default();
                Ease {
                    speed: Some(speed * scale.get(axis).copied().unwrap_or(1.0)),
                    influence: Some(influence * 100.0),
                }
            })
            .collect()
    };
    key.in_ease = Some(side(1));
    key.out_ease = Some(side(3));
}

/// A dimension-separated property keeps its animation on one follower per axis,
/// and the DOM reports the followers under the leader as well as beside it. So
/// once a group is read, a separated leader collects its followers.
pub(super) fn fill_separated(properties: &mut [Property], separated: &[String]) {
    if separated.is_empty() {
        return;
    }
    let followers: Vec<(String, Property)> = properties
        .iter()
        .filter_map(|node| {
            let name = node.match_name.clone()?;
            name.rsplit_once('_')
                .filter(|(_, axis)| axis.len() == 1 && axis.chars().all(|c| c.is_ascii_digit()))
                .map(|(leader, _)| (leader.to_string(), node.clone()))
        })
        .collect();

    for node in properties.iter_mut() {
        let Some(name) = node.match_name.clone() else {
            continue;
        };
        if !separated.contains(&name) {
            continue;
        }
        let mine: Vec<Property> = followers
            .iter()
            .filter(|(leader, _)| *leader == name)
            .map(|(_, follower)| follower.clone())
            .collect();
        if mine.is_empty() {
            continue;
        }
        // Scripting does not report the leader's own keys once the dimensions
        // are separated: the followers are the animation, and reading the
        // leader is how a moving layer imports as a still one.
        node.keyframes = None;
        node.separated = Some(mine);
    }
}
