//! A plugin's window is asked with its own values.

use super::*;

/// **A plugin is asked for its frames with the instance's own values.** A
/// retimer's window can follow a setting, and the question used to go out with
/// the declared defaults, so every copy of the effect answered for a plugin
/// nobody had touched and the frames it then read were never fetched.
#[test]
fn a_plugins_window_is_asked_with_its_own_values() {
    /// Reads `radius` frames either side of its own.
    struct Windowed;

    impl PluginHost for Windowed {
        fn render(
            &self,
            _instance: uuid::Uuid,
            _time: f64,
            _params: &ParamSnapshot,
            source: Frame16,
            _neighbours: &[(i32, Frame16)],
        ) -> Rendering {
            Rendering {
                frame: source,
                error: None,
            }
        }

        fn frames_needed(
            &self,
            _instance: uuid::Uuid,
            _time: f64,
            params: &ParamSnapshot,
        ) -> Option<Vec<i32>> {
            let Some(PropValue::Double(radius)) = params.get("radius") else {
                return None;
            };
            let radius = radius.first().copied()? as i32;
            Some((-radius..=radius).collect())
        }

        fn press(
            &self,
            _instance: uuid::Uuid,
            _time: f64,
            _params: &ParamSnapshot,
            _name: &str,
            _source: Frame16,
        ) -> Result<crate::instance::Settled, String> {
            Err("not this test".to_owned())
        }
    }

    let mut props = PropertySet::new();
    props.seed(crate::ffi::prop_keys::PARAM_DEFAULT, PropValue::double(1.0));
    let descriptor = PluginDescriptor {
        identifier: "test.window".to_owned(),
        version: (1, 0),
        grouping: String::new(),
        label: "Window test plugin".to_owned(),
        contexts: vec![Context::Filter],
        params: vec![crate::describe::ParamDescription {
            name: "radius".to_owned(),
            param_type: crate::ffi::param_types::DOUBLE.to_owned(),
            props,
        }],
        clips: Vec::new(),
        temporal: true,
        render_thread_safety: None,
    };
    let schema: &'static EffectSchema = Box::leak(Box::new(
        crate::schema::schema_of(&descriptor).expect("a schema"),
    ));
    let def = OfxEffectDef::new(&descriptor, schema, std::sync::Arc::new(Windowed));
    let instance = |radius: Option<f64>| lumit_core::model::EffectInstance {
        id: uuid::Uuid::now_v7(),
        effect: lumit_core::model::EffectKey {
            namespace: lumit_core::model::EffectNamespace::Ofx,
            match_name: schema.match_name.to_owned(),
            version: 1,
            extra: serde_json::Map::new(),
        },
        enabled: true,
        params: radius
            .map(|radius| lumit_core::model::EffectParam {
                id: "radius".to_owned(),
                value: EffectValue::Float(lumit_core::anim::Property::fixed(radius)),
                extra: serde_json::Map::new(),
            })
            .into_iter()
            .collect(),
        sample_temporally: true,
        custom_name: None,
        linked_pairs: Vec::new(),
        plugin_state: None,
        roto: None,
        extra: serde_json::Map::new(),
    };

    assert_eq!(
        def.frames_needed(&instance(None), 0.0),
        Some(vec![-1, 0, 1]),
        "an instance nobody has touched is at the plugin's default"
    );
    assert_eq!(
        def.frames_needed(&instance(Some(3.0)), 0.0),
        Some(vec![-3, -2, -1, 0, 1, 2, 3]),
        "and one with its own radius is asked with that radius"
    );
}
