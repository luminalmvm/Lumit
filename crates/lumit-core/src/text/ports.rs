//! A text animator's numbers as ports on its layer's Source box.

use super::{glyph_xforms_near, GlyphXform, SelectorKind, SpokenWord, TextAnimator};
use crate::anim::Property;

/// What a text animator's numbers are called on its layer's Source box:
/// `text.<animator>.<number>`, the animator by its place in the list.
pub const TEXT_PORT_PREFIX: &str = "text.";

/// The Source box port of animator `index`'s number `name`.
#[must_use]
pub fn animator_port(index: usize, name: &str) -> String {
    format!("{TEXT_PORT_PREFIX}{index}.{name}")
}

/// The animator and number a Source box port names.
#[must_use]
pub fn parse_animator_port(port: &str) -> Option<(usize, &str)> {
    let rest = port.strip_prefix(TEXT_PORT_PREFIX)?;
    let (index, name) = rest.split_once('.')?;
    Some((index.parse().ok()?, name))
}

pub(super) fn is_text_port(to: &crate::graph::InputRef) -> bool {
    matches!(to, crate::graph::InputRef::Param { node: crate::graph::NodeRef::Source, port }
        if port.starts_with(TEXT_PORT_PREFIX))
}

impl TextAnimator {
    /// The keyable numbers a driver can be wired into, by the name its port
    /// carries and as the panels call them: the selector's own, the property
    /// groups the animator lists, and its font axes (`axis.<tag>`).
    #[must_use]
    pub fn wireable_numbers(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        let mut add = |name: &str, label: &str| out.push((name.to_owned(), label.to_owned()));
        // Each selector's own numbers: the first as they always were, a
        // later one as `s<k>.<number>`, named after its place in the list.
        for (k, selector) in self.all_selectors().enumerate() {
            for (name, label) in selector_numbers(selector.kind) {
                if k == 0 {
                    add(name, label);
                } else {
                    add(
                        &format!("s{k}.{name}"),
                        &format!("Selector {} · {label}", k + 1),
                    );
                }
            }
        }
        // The groups that came later are listed only once added, even on an
        // animator from before the list, so an old animator shows no more
        // than it did.
        let late = ["skew", "tracking", "blur", "character_offset"];
        let shows = |group: &str| match &self.properties {
            None => !late.contains(&group),
            Some(list) => list.iter().any(|p| p == group),
        };
        for (group, numbers) in [
            (
                "position",
                &[("position_x", "Position x"), ("position_y", "Position y")][..],
            ),
            ("rotation", &[("rotation", "Rotation")][..]),
            (
                "scale",
                &[("scale_x", "Scale x"), ("scale_y", "Scale y")][..],
            ),
            ("opacity", &[("opacity", "Opacity")][..]),
            ("skew", &[("skew", "Skew"), ("skew_axis", "Skew axis")][..]),
            ("tracking", &[("tracking", "Tracking amount")][..]),
            ("blur", &[("blur", "Blur")][..]),
            (
                "character_offset",
                &[("character_offset", "Character offset")][..],
            ),
            (
                "fill",
                &[
                    ("fill_r", "Fill offset r"),
                    ("fill_g", "Fill offset g"),
                    ("fill_b", "Fill offset b"),
                ][..],
            ),
        ] {
            if shows(group) {
                for (name, label) in numbers {
                    add(name, label);
                }
            }
        }
        out.push(("animator_amount".to_owned(), "Animator amount".to_owned()));
        for axis in &self.axes {
            out.push((
                format!("axis.{}", axis.tag),
                format!("Font axis {}", axis.tag),
            ));
        }
        out
    }

    /// The keyable number `name` names, the way [`Self::wireable_numbers`]
    /// calls it.
    pub fn number_mut(&mut self, name: &str) -> Option<&mut Property> {
        if let Some(tag) = name.strip_prefix("axis.") {
            return self
                .axes
                .iter_mut()
                .find(|a| a.tag == tag)
                .map(|a| &mut a.value);
        }
        // A later selector's number, `s<k>.<number>`.
        let (k, name) = match name.strip_prefix('s').and_then(|rest| rest.split_once('.')) {
            Some((k, rest)) if !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit()) => {
                (k.parse::<usize>().ok()?, rest)
            }
            _ => (0, name),
        };
        if k > 0 {
            let s = self.selector_at_mut(k)?;
            if !selector_numbers(s.kind).iter().any(|(n, _)| *n == name) {
                return None;
            }
            return Some(match name {
                "start" => &mut s.start,
                "end" => &mut s.end,
                "offset" => &mut s.offset,
                "radius" => &mut s.radius,
                "min_amount" => &mut s.min_amount,
                "max_amount" => &mut s.max_amount,
                "amount" => &mut s.amount,
                "delay" => &mut s.delay,
                "delay_ease" => &mut s.delay_ease,
                "bounce_amplitude" => &mut s.bounce_amplitude,
                "bounce_frequency" => &mut s.bounce_frequency,
                "bounce_decay" => &mut s.bounce_decay,
                "wiggles" => &mut s.wiggles,
                "correlation" => &mut s.correlation,
                _ => return None,
            });
        }
        let s = &mut self.selector;
        Some(match name {
            "start" => &mut s.start,
            "end" => &mut s.end,
            "offset" => &mut s.offset,
            "radius" => &mut s.radius,
            "min_amount" => &mut s.min_amount,
            "max_amount" => &mut s.max_amount,
            "amount" => &mut s.amount,
            "delay" => &mut s.delay,
            "delay_ease" => &mut s.delay_ease,
            "bounce_amplitude" => &mut s.bounce_amplitude,
            "bounce_frequency" => &mut s.bounce_frequency,
            "bounce_decay" => &mut s.bounce_decay,
            "wiggles" => &mut s.wiggles,
            "correlation" => &mut s.correlation,
            "position_x" => &mut self.position_x,
            "position_y" => &mut self.position_y,
            "position_z" => &mut self.position_z,
            "rotation" => &mut self.rotation,
            "rotation_x" => &mut self.rotation_x,
            "rotation_y" => &mut self.rotation_y,
            "scale_x" => &mut self.scale_x,
            "scale_y" => &mut self.scale_y,
            "opacity" => &mut self.opacity,
            "fill_r" => &mut self.fill_r,
            "fill_g" => &mut self.fill_g,
            "fill_b" => &mut self.fill_b,
            "skew" => &mut self.skew,
            "skew_axis" => &mut self.skew_axis,
            "tracking" => &mut self.tracking,
            "blur" => &mut self.blur,
            "character_offset" => &mut self.character_offset,
            "animator_amount" => &mut self.amount,
            _ => return None,
        })
    }
}

/// What each character of `layer`'s words `line` is asked to do at comp time
/// `t_comp`. The same as [`super::glyph_xforms`], with every Proximity selector
/// measured against where its target layer is at that moment.
#[must_use]
pub fn layer_glyph_xforms(
    doc: &std::sync::Arc<crate::model::Document>,
    comp: &crate::model::Composition,
    layer: &crate::model::Layer,
    document: &crate::model::TextDocument,
    line: &str,
    t_comp: f64,
    spoken: &[SpokenWord],
) -> Vec<GlyphXform> {
    let lt = crate::time::layer_time(t_comp, layer.start_offset.0);
    let near = super::selector::near_of(doc, comp, layer, document, line, t_comp);
    // A number a driver is wired into on the layer's Source box follows the
    // driver instead of its own keys.
    let driven = if layer.graph.edges.iter().any(|e| is_text_port(&e.to)) {
        let context = std::sync::Arc::new(crate::expression::ExpressionContext {
            document: doc.clone(),
            comp: Some(comp.id),
            layer: Some(layer.id),
            comp_time: t_comp,
            current_depth: 0,
            inputs: None,
        });
        crate::fx::drivers::driven_source_params(&layer.graph, lt, context, None)
    } else {
        Vec::new()
    };
    if driven.is_empty() {
        return glyph_xforms_near(&document.animators, line, lt, spoken, &near);
    }
    let mut animators = document.animators.clone();
    for (port, value) in driven {
        let Some((index, name)) = parse_animator_port(&port) else {
            continue;
        };
        if let Some(p) = animators.get_mut(index).and_then(|a| a.number_mut(name)) {
            *p = Property::fixed(value);
        }
    }
    glyph_xforms_near(&animators, line, lt, spoken, &near)
}

/// A selector's wireable numbers, by port name and label.
fn selector_numbers(kind: SelectorKind) -> &'static [(&'static str, &'static str)] {
    match kind {
        SelectorKind::Range => &[
            ("start", "Range start"),
            ("end", "Range end"),
            ("offset", "Range offset"),
        ],
        SelectorKind::Proximity => &[("radius", "Radius")],
        SelectorKind::Wiggly => &[
            ("max_amount", "Max amount"),
            ("min_amount", "Min amount"),
            ("wiggles", "Wiggles/second"),
            ("correlation", "Correlation"),
        ],
        SelectorKind::Stagger => &[
            ("amount", "Amount"),
            ("delay", "Delay"),
            ("delay_ease", "Delay ease"),
            ("bounce_amplitude", "Bounce amplitude"),
            ("bounce_frequency", "Bounce frequency"),
            ("bounce_decay", "Bounce decay"),
        ],
    }
}

/// Where each of `old`'s animators is in `new`, or `None` for one that is
/// gone. Animators are matched by name, the n-th of a name to the n-th of it
/// (a preset may lay two of one name); those left over on both sides, when as
/// many are left on each, are taken as renamed, in order.
#[must_use]
pub fn animator_moves(old: &[TextAnimator], new: &[TextAnimator]) -> Vec<Option<usize>> {
    let nth = |list: &[TextAnimator], i: usize| {
        list[..i].iter().filter(|a| a.name == list[i].name).count()
    };
    let mut taken = vec![false; new.len()];
    let mut out: Vec<Option<usize>> = (0..old.len())
        .map(|i| {
            let n = nth(old, i);
            let j = (0..new.len()).find(|&j| new[j].name == old[i].name && nth(new, j) == n)?;
            taken[j] = true;
            Some(j)
        })
        .collect();
    let lost: Vec<usize> = (0..old.len()).filter(|&i| out[i].is_none()).collect();
    let fresh: Vec<usize> = (0..new.len()).filter(|&j| !taken[j]).collect();
    if lost.len() == fresh.len() {
        for (i, j) in lost.into_iter().zip(fresh) {
            out[i] = Some(j);
        }
    }
    out
}

/// Where each of `old`'s selectors is in `new`, the same animator before and
/// after an edit: one fewer is the first that differs taken out, more are
/// added at the end.
fn selector_moves(old: &TextAnimator, new: &TextAnimator) -> Vec<Option<usize>> {
    let (before, after) = (old.all_selectors().count(), new.all_selectors().count());
    if after + 1 == before {
        let gone = (0..before)
            .find(|&k| old.selector_at(k) != new.selector_at(k))
            .unwrap_or(before - 1);
        return (0..before)
            .map(|k| match k.cmp(&gone) {
                std::cmp::Ordering::Less => Some(k),
                std::cmp::Ordering::Equal => None,
                std::cmp::Ordering::Greater => Some(k - 1),
            })
            .collect();
    }
    (0..before).map(|k| (k < after).then_some(k)).collect()
}

/// A selector's number named on a port: which selector, and the number.
fn selector_of(name: &str) -> Option<(usize, &str)> {
    let is_selector = |n: &str| {
        [
            SelectorKind::Range,
            SelectorKind::Proximity,
            SelectorKind::Wiggly,
            SelectorKind::Stagger,
        ]
        .iter()
        .any(|&kind| selector_numbers(kind).iter().any(|(p, _)| *p == n))
    };
    if let Some((k, rest)) = name.strip_prefix('s').and_then(|r| r.split_once('.')) {
        if !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit()) {
            return Some((k.parse().ok()?, rest));
        }
    }
    is_selector(name).then_some((0, name))
}

/// Move the wires into a Text layer's animator numbers along with the
/// animators, when its list goes from `old` to `new`: a wire follows its
/// animator (and selector) to its new place, and one into an animator or a
/// selector that is gone is taken off. True when anything changed.
pub fn remap_text_ports(
    graph: &mut crate::graph::LayerGraph,
    old: &[TextAnimator],
    new: &[TextAnimator],
) -> bool {
    use crate::graph::{InputRef, NodeRef};
    let moves = animator_moves(old, new);
    let mut changed = false;
    graph.edges.retain_mut(|edge| {
        let InputRef::Param {
            node: NodeRef::Source,
            port,
        } = &mut edge.to
        else {
            return true;
        };
        let Some((index, name)) = parse_animator_port(port) else {
            return true;
        };
        // A wire into no animator stays as it was found.
        let Some(&moved) = moves.get(index) else {
            return true;
        };
        let Some(to) = moved else {
            changed = true;
            return false;
        };
        let name = match selector_of(name) {
            Some((k, number)) => {
                let Some(Some(k2)) = selector_moves(&old[index], &new[to]).get(k).copied() else {
                    changed = true;
                    return false;
                };
                // A selector turned into another kind has no such number: the
                // wire would lead nowhere, so it is taken off.
                let has = new[to]
                    .selector_at(k2)
                    .is_some_and(|s| selector_numbers(s.kind).iter().any(|(n, _)| *n == number));
                if !has {
                    changed = true;
                    return false;
                }
                if k2 == 0 {
                    number.to_owned()
                } else {
                    format!("s{k2}.{number}")
                }
            }
            None => name.to_owned(),
        };
        let next = animator_port(to, &name);
        if next != *port {
            *port = next;
            changed = true;
        }
        true
    });
    changed
}
