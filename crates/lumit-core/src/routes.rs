//! Where a layer's sound goes: the master, or a Bus layer, and the sends
//! that copy it to other buses on the way.
//!
//! A **Bus layer** is a Null layer with `audio_only` set. It has no sound of
//! its own: its rack and its Volume work on what is routed to it. A flag and
//! not a kind of its own, so an older Lumit opens the file and sees a silent
//! Null.

use crate::anim::Property;
use crate::mixer::Duck;
use crate::model::{Composition, Layer, LayerKind};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A layer's routing. The default is the master with no sends, which is every
/// layer in a project saved before buses existed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioRoutes {
    /// The bus this layer's own sound goes to instead of the master. A bus that
    /// is gone, or is not a bus, means the master.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sends: Vec<AudioSend>,
}

impl AudioRoutes {
    #[must_use]
    pub fn is_master(&self) -> bool {
        self.output.is_none() && self.sends.is_empty()
    }
}

/// A copy of the layer's sound sent to a bus, after its rack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioSend {
    pub bus: Uuid,
    /// How much is sent, in dB like Volume, and keyable like it.
    #[serde(default = "Property::zero")]
    pub level_db: Property,
    /// Sent from before the layer's Volume and Pan rather than after them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pre: bool,
}

impl Layer {
    /// This layer is a Bus layer.
    #[must_use]
    pub fn is_bus(&self) -> bool {
        self.audio_only && matches!(self.kind, LayerKind::Null)
    }
}

/// Why a layer's routing was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RouteError {
    #[error("the output is not a bus of this composition")]
    OutputNotABus,
    #[error("a send goes to something that is not a bus of this composition")]
    SendNotABus,
    #[error("a layer cannot be routed to itself")]
    ToItself,
    #[error("two sends go to the same bus")]
    TwoSendsToOneBus,
    #[error("the layer it lowers behind is not in this composition")]
    UnknownKey,
    #[error("the sound would go round in a circle")]
    Cycle,
}

/// Whether `layer` in `comp` may take `routes` and lower behind `duck`.
///
/// The output and every send must be a Bus layer of this comp, each bus sent
/// to once, and nothing may come back round: a bus feeding a bus that feeds
/// it, or a layer lowered behind something its own sound reaches, would never
/// finish mixing. An id the layer already names is let through when its layer
/// has since been deleted, since that reads as the master and must not stop
/// every later edit to the routing.
pub fn check_routes(
    comp: &Composition,
    layer: Uuid,
    routes: &AudioRoutes,
    duck: Option<&Duck>,
) -> Result<(), RouteError> {
    let now = comp.layers.iter().find(|l| l.id == layer);
    let named = |id: Uuid| {
        now.is_some_and(|l| {
            l.audio_routes.output == Some(id)
                || l.audio_routes.sends.iter().any(|s| s.bus == id)
                || l.duck.is_some_and(|d| d.behind == id)
        })
    };
    let found = |id: Uuid| comp.layers.iter().find(|l| l.id == id);
    let is_bus = |id: Uuid| found(id).map_or_else(|| named(id), Layer::is_bus);
    if let Some(output) = routes.output {
        if output == layer {
            return Err(RouteError::ToItself);
        }
        if !is_bus(output) {
            return Err(RouteError::OutputNotABus);
        }
    }
    for (at, send) in routes.sends.iter().enumerate() {
        if send.bus == layer {
            return Err(RouteError::ToItself);
        }
        if !is_bus(send.bus) {
            return Err(RouteError::SendNotABus);
        }
        if routes.sends[..at].iter().any(|s| s.bus == send.bus) {
            return Err(RouteError::TwoSendsToOneBus);
        }
    }
    if let Some(duck) = duck {
        if duck.behind == layer {
            return Err(RouteError::ToItself);
        }
        if found(duck.behind).is_none() && !named(duck.behind) {
            return Err(RouteError::UnknownKey);
        }
    }
    let edges = edges(comp, Some((layer, routes, duck)));
    if edges.iter().any(|&(from, to)| reaches(&edges, to, from)) {
        return Err(RouteError::Cycle);
    }
    Ok(())
}

/// Whether `layer` may send to or output to `bus` as the comp stands, for a
/// menu to ask before it offers the bus.
#[must_use]
pub fn route_allowed(comp: &Composition, layer: Uuid, bus: Uuid) -> bool {
    bus != layer
        && comp.layers.iter().any(|l| l.id == bus && l.is_bus())
        && !reaches(&edges(comp, None), bus, layer)
}

/// Every way one layer of `comp` reaches another: its sound into a bus, and a
/// layer's sound into whatever lowers behind it. `instead` stands in for one
/// layer's own routing and duck.
fn edges(
    comp: &Composition,
    instead: Option<(Uuid, &AudioRoutes, Option<&Duck>)>,
) -> Vec<(Uuid, Uuid)> {
    let mut edges = Vec::new();
    for l in &comp.layers {
        let (routes, duck) = match instead {
            Some((id, routes, duck)) if id == l.id => (routes, duck),
            _ => (&l.audio_routes, l.duck.as_ref()),
        };
        edges.extend(routes.output.map(|bus| (l.id, bus)));
        edges.extend(routes.sends.iter().map(|s| (l.id, s.bus)));
        edges.extend(duck.map(|d| (d.behind, l.id)));
    }
    edges
}

/// Whether `to` can be reached from `from` along `edges`.
fn reaches(edges: &[(Uuid, Uuid)], from: Uuid, to: Uuid) -> bool {
    let mut seen = vec![from];
    let mut next = vec![from];
    while let Some(at) = next.pop() {
        if at == to {
            return true;
        }
        for &(a, b) in edges {
            if a == at && !seen.contains(&b) {
                seen.push(b);
                next.push(b);
            }
        }
    }
    false
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::model::LinearColour;
    use crate::time::{Duration, FrameRate, Rational};

    fn bus(comp: &Composition) -> Layer {
        let mut layer = crate::edits::null_layer(comp);
        layer.audio_only = true;
        layer
    }

    /// A circle through the routing would never finish mixing, so it is
    /// refused: a bus feeding the bus that feeds it, and a layer lowered
    /// behind a sound its own sound reaches.
    #[test]
    fn a_route_or_a_duck_that_comes_back_round_is_refused() {
        let mut comp = Composition {
            graph: None,
            master_volume_db: 0.0,
            master_keys: None,
            master_automation: Default::default(),
            loudness_target: None,
            lighting: Default::default(),
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
            id: Uuid::now_v7(),
            name: "Mix".into(),
            width: 64,
            height: 64,
            frame_rate: FrameRate::new(25, 1).unwrap(),
            duration: Duration(Rational::new(4, 1).unwrap()),
            background: LinearColour([0.0, 0.0, 0.0, 1.0]),
            work_area: None,
            layers: Vec::new(),
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let (a, b) = (bus(&comp), bus(&comp));
        let (a_id, b_id) = (a.id, b.id);
        let mut voice = crate::edits::null_layer(&comp);
        voice.audio_routes.output = Some(a_id);
        let voice_id = voice.id;
        let music_id = Uuid::now_v7();
        let mut music = crate::edits::null_layer(&comp);
        music.id = music_id;
        comp.layers = vec![a, b, voice, music];
        let to = |bus: Uuid| AudioRoutes {
            output: Some(bus),
            sends: Vec::new(),
        };

        // A bus may feed another bus, but not one that feeds it back.
        assert_eq!(check_routes(&comp, a_id, &to(b_id), None), Ok(()));
        comp.layers[0].audio_routes = to(b_id);
        assert_eq!(
            check_routes(&comp, b_id, &to(a_id), None),
            Err(RouteError::Cycle)
        );
        assert!(!route_allowed(&comp, b_id, a_id));
        assert!(route_allowed(&comp, voice_id, b_id));

        // The music may lower behind the voice, but the voice may not then
        // lower behind the music: each would wait on the other.
        let behind = |id: Uuid| crate::mixer::Duck::behind(id);
        assert_eq!(
            check_routes(
                &comp,
                music_id,
                &AudioRoutes::default(),
                Some(&behind(voice_id))
            ),
            Ok(())
        );
        comp.layers[3].duck = Some(behind(voice_id));
        assert_eq!(
            check_routes(&comp, voice_id, &to(a_id), Some(&behind(music_id))),
            Err(RouteError::Cycle)
        );
    }
}
