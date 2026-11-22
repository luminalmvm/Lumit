//! Rows the plugin marks secret.

use super::*;

/// A parameter the plugin marks secret keeps its row and starts hidden, and
/// so does everything inside a secret group. Once the plugin has answered an
/// edit the rows follow what it reports, kept on the instance: spektrafilm
/// shows its HDR output rows only once the output role says HDR, and Resolve
/// draws whatever is not secret now.
#[test]
fn a_secret_parameter_is_a_hidden_row_until_the_plugin_says_otherwise() {
    struct Reporting(std::collections::BTreeSet<String>);

    impl PluginHost for Reporting {
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

        fn settle(
            &self,
            _instance: uuid::Uuid,
            _made_with: &ParamSnapshot,
            handed: &ParamSnapshot,
        ) -> Result<crate::instance::Settled, String> {
            Ok(crate::instance::Settled {
                params: handed.clone(),
                controls: crate::instance::Controls {
                    secret: self.0.clone(),
                    ..crate::instance::Controls::default()
                },
            })
        }

        fn frames_needed(
            &self,
            _instance: uuid::Uuid,
            _time: f64,
            _params: &ParamSnapshot,
        ) -> Option<Vec<i32>> {
            None
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

    let param = |name: &str, kind: &str, secret: bool, parent: &str| {
        let mut props = PropertySet::new();
        props.seed(keys::PARAM_DEFAULT, PropValue::double(0.5));
        props.seed(keys::PARAM_SECRET, PropValue::int(i32::from(secret)));
        if !parent.is_empty() {
            props.seed(
                keys::PARAM_PARENT,
                PropValue::string(parent).expect("a name"),
            );
        }
        crate::describe::ParamDescription {
            name: name.to_owned(),
            param_type: kind.to_owned(),
            props,
        }
    };
    let descriptor = PluginDescriptor {
        identifier: "test.secret".to_owned(),
        version: (1, 0),
        grouping: String::new(),
        label: "Secret test plugin".to_owned(),
        contexts: vec![Context::Filter],
        params: vec![
            param("shown", crate::ffi::param_types::DOUBLE, false, ""),
            param("hidden", crate::ffi::param_types::DOUBLE, true, ""),
            param("advanced", crate::ffi::param_types::GROUP, true, ""),
            param("inside", crate::ffi::param_types::DOUBLE, false, "advanced"),
            param("deeper", crate::ffi::param_types::GROUP, false, "advanced"),
            param("nested", crate::ffi::param_types::DOUBLE, false, "deeper"),
        ],
        clips: Vec::new(),
        temporal: false,
        render_thread_safety: None,
    };
    let schema: &'static EffectSchema = Box::leak(Box::new(
        crate::schema::schema_of(&descriptor).expect("a schema"),
    ));
    let ids: Vec<&str> = schema.params.iter().map(|row| row.id).collect();
    assert_eq!(
        ids,
        vec!["shown", "hidden", "inside", "nested"],
        "every row is in the schema, hidden or not"
    );

    // The plugin will report the opposite of what it declared.
    let live = std::collections::BTreeSet::from(["shown".to_owned()]);
    let host = std::sync::Arc::new(Reporting(live));
    let def = OfxEffectDef::new(&descriptor, schema, host);
    let mut inst = lumit_core::model::EffectInstance {
        id: uuid::Uuid::now_v7(),
        effect: lumit_core::model::EffectKey {
            namespace: lumit_core::model::EffectNamespace::Ofx,
            match_name: schema.match_name.to_owned(),
            version: 1,
            extra: serde_json::Map::new(),
        },
        enabled: true,
        params: Vec::new(),
        sample_temporally: false,
        custom_name: None,
        linked_pairs: Vec::new(),
        plugin_state: None,
        roto: None,
        extra: serde_json::Map::new(),
    };
    assert_eq!(
        def.row_state(&inst).hidden,
        vec!["hidden", "inside", "nested"],
        "before the plugin has answered, the describe-time flags, groups included"
    );

    let answer = def.settle(None, &inst).expect("the plugin answered");
    inst.set_plugin_state(&answer.memory.expect("its controls are kept"));
    assert_eq!(
        def.row_state(&inst).hidden,
        vec!["shown"],
        "after an edit, what the plugin reported"
    );
    inst.id = uuid::Uuid::now_v7();
    inst.plugin_state = None;
    assert_eq!(
        def.row_state(&inst).hidden,
        vec!["hidden", "inside", "nested"],
        "another instance has not been answered and starts from describe"
    );
}
