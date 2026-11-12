//! A text animator's numbers as ports on its layer's Source box.

use super::{SelectorKind, TextAnimator};
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
        match self.selector.kind {
            SelectorKind::Range => {
                add("start", "Range start");
                add("end", "Range end");
                add("offset", "Range offset");
            }
            SelectorKind::Proximity => add("radius", "Radius"),
            SelectorKind::Wiggly => {
                add("max_amount", "Max amount");
                add("min_amount", "Min amount");
                add("wiggles", "Wiggles/second");
                add("correlation", "Correlation");
            }
        }
        let shows = |group: &str| {
            self.properties
                .as_ref()
                .is_none_or(|list| list.iter().any(|p| p == group))
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
        let s = &mut self.selector;
        Some(match name {
            "start" => &mut s.start,
            "end" => &mut s.end,
            "offset" => &mut s.offset,
            "radius" => &mut s.radius,
            "min_amount" => &mut s.min_amount,
            "max_amount" => &mut s.max_amount,
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
            _ => return None,
        })
    }
}
