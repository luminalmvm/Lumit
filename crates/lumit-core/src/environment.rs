//! The Environment light: a picture of everything round the scene, wrapped
//! round it, that lights its 3D layers and models and can show behind them.
//! The picture is another layer of the comp, read as a panorama twice as wide
//! as it is tall, and the light's own rotation turns it.

use crate::anim::Property;
use crate::model::{
    Composition, Layer, LayerKind, LightDef, LightKind, Read, Solves, TransformProp,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// An Environment light's own rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LightEnvironment {
    /// The layer whose picture surrounds the scene.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Uuid>,
    /// Shown behind everything, as the camera sees it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub background: bool,
    /// Percent: how soft it is there.
    #[serde(default = "Property::zero")]
    pub blur: Property,
}

impl Default for LightEnvironment {
    fn default() -> Self {
        Self {
            source: None,
            background: false,
            blur: Property::zero(),
        }
    }
}

/// The part of [`LightEnvironment`] the choice rows set, one op at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EnvironmentSettings {
    #[serde(default)]
    pub source: Option<Uuid>,
    #[serde(default)]
    pub background: bool,
}

impl LightEnvironment {
    /// Left out of a saved light while it is untouched.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    #[must_use]
    pub fn settings(&self) -> EnvironmentSettings {
        EnvironmentSettings {
            source: self.source,
            background: self.background,
        }
    }

    pub fn set_settings(&mut self, s: EnvironmentSettings) {
        self.source = s.source;
        self.background = s.background;
    }

    #[must_use]
    pub fn channel(&self, prop: TransformProp) -> Option<&Property> {
        (prop == TransformProp::BackgroundBlur).then_some(&self.blur)
    }

    pub fn channel_mut(&mut self, prop: TransformProp) -> Option<&mut Property> {
        (prop == TransformProp::BackgroundBlur).then_some(&mut self.blur)
    }
}

/// The comp's environment at one moment, as the renderer takes it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedEnvironment {
    /// The layer whose picture it is.
    pub source: Uuid,
    /// Scene-linear, the intensity in it.
    pub colour: [f32; 3],
    /// The light's own turn in degrees, in the compositor's `Ry · Rx · Rz`
    /// order, its parents and aim in it.
    pub rotation_deg: (f64, f64, f64),
    pub background: bool,
    /// How soft the background is, 0 to 1.
    pub blur: f32,
}

impl Composition {
    /// The Environment light that wins at `t`: the topmost one that is
    /// visible, in its span and has a source.
    fn environment_light(&self, t: f64) -> Option<(&Layer, &LightDef, Uuid)> {
        self.layers.iter().find_map(|l| {
            let LayerKind::Light { light } = &l.kind else {
                return None;
            };
            let in_span = t >= l.in_point.0.to_f64() && t < l.out_point.0.to_f64();
            let lit = light.kind == LightKind::Environment && l.switches.visible && in_span;
            Some((l, light.as_ref(), light.environment.source.filter(|_| lit)?))
        })
    }

    /// The layer whose picture lights the comp at `t`, which is read whether
    /// it is drawn or not.
    #[must_use]
    pub fn environment_source(&self, t: f64) -> Option<Uuid> {
        self.environment_light(t).map(|(_, _, source)| source)
    }

    /// Whether `layer` is any Environment light's picture, lit or not, and
    /// so never drawn itself.
    #[must_use]
    pub fn feeds_environment(&self, layer: Uuid) -> bool {
        self.layers.iter().any(|l| {
            matches!(&l.kind, LayerKind::Light { light }
                if light.kind == LightKind::Environment && light.environment.source == Some(layer))
        })
    }

    /// The environment at `t`, from [`Self::environment_source`]'s light.
    /// `None` when there is none, which is every comp made before there were
    /// any.
    #[must_use]
    pub fn environment_at(
        &self,
        t: f64,
        read: &Read<'_>,
        solves: Solves<'_>,
    ) -> Option<ResolvedEnvironment> {
        let (l, light, source) = self.environment_light(t)?;
        let lt = crate::time::layer_time(t, l.start_offset.0);
        // A surround can't take light away, or a float export would hold
        // negative pixels behind everything.
        let gain = (light.intensity.value_at(lt) / 100.0).max(0.0) as f32;
        // Its own turn and its parents', never an aim, which would tilt the
        // picture by where the light happens to stand.
        let own = l.transform.placement_at(lt, false, &|p| read(l, p));
        let eye = crate::camera::Eye {
            position: own.position,
            rotation_deg: own.rotation_deg,
            point_of_interest: None,
        };
        let eye =
            crate::model::parent_space(self, l, t, read, solves).map_or(eye, |s| s.carry(eye));
        Some(ResolvedEnvironment {
            source,
            colour: light
                .colour
                .each_ref()
                .map(|c| (c.value_at(lt) as f32).max(0.0) * gain),
            rotation_deg: eye.rotation_deg,
            background: light.environment.background,
            blur: (light.environment.blur.value_at(lt) / 100.0).clamp(0.0, 1.0) as f32,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::anim::Property;
    use crate::model::*;
    use crate::time::{Duration, FrameRate, Rational};

    fn environment(source: uuid::Uuid) -> Layer {
        let mut def = LightDef {
            kind: LightKind::Environment,
            ..LightDef::default()
        };
        def.environment.source = Some(source);
        def.environment.background = true;
        def.environment.blur = Property::fixed(25.0);
        let kind = LayerKind::Light {
            light: Box::new(def),
        };
        let ten = Rational::new(10, 1).unwrap();
        crate::edits::base_layer("Environment".into(), kind, ten, TransformGroup::default())
    }

    /// A saved light keeps its picture, and the topmost one that shows wins.
    #[test]
    fn an_environment_light_keeps_its_source_and_the_top_one_wins() {
        let (top, under) = (uuid::Uuid::now_v7(), uuid::Uuid::now_v7());
        let light = environment(top);
        let opened: Layer = serde_json::from_str(&serde_json::to_string(&light).unwrap()).unwrap();
        assert_eq!(opened, light);
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
            id: uuid::Uuid::now_v7(),
            name: "environment".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour([0.0, 0.0, 0.0, 1.0]),
            work_area: None,
            layers: vec![opened, environment(under)],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let read = |_: &Layer, p: &Property| p.value_at(1.0);
        let e = comp.environment_at(1.0, &read, None).unwrap();
        assert_eq!((e.source, e.background, e.blur), (top, true, 0.25));
        assert!(comp.lights_at(1.0, &read, None).is_empty());
        comp.layers[0].switches.visible = false;
        assert_eq!(comp.environment_at(1.0, &read, None).unwrap().source, under);
    }
}
