//! Tests for effects that join the catalogue at run time, as a plugin does.

use super::*;

// ------------------------------------------------- the run-time catalogue --

/// A definition that arrived at run time, as a plugin's does. Declared
/// here rather than driven by a real OFX bundle because what is under test is
/// the *seam*: `lumit-core` cannot depend on the plugin host (docs/05), so what
/// it can be shown is that an effect it never compiled in becomes an effect like
/// any other. `lumit-ofx` proves the other half — that a real plugin makes one
/// of these.
struct RegisteredDef {
    schema: &'static EffectSchema,
    /// The frames the instance reads, as `frames_needed` answers them. `None` is
    /// every effect but a retimer.
    window: Option<Vec<i32>>,
}

impl EffectDef for RegisteredDef {
    fn schema(&self) -> &'static EffectSchema {
        self.schema
    }
    fn frames_needed(&self, _inst: &EffectInstance, _lt: f64) -> Option<Vec<i32>> {
        self.window.clone()
    }
}

/// A leaked declaration under a plugin-shaped name, exactly as the OFX host
/// leaks one for a plugin it has just described.
fn a_registered_schema(
    match_name: &'static str,
    temporal: &'static [i32],
) -> &'static EffectSchema {
    Box::leak(Box::new(EffectSchema {
        match_name,
        label: "Registered",
        version: 1,
        category: FxCategory::Utility,
        traits: EffectTraits {
            cost: CostClass::Heavy,
            roi: Roi::FullFrame,
            temporal,
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[],
        groups: &[],
        enabled_when: &[],
        matte: MatteRole::None,
    }))
}

/// An instance of a registered effect, in the namespace a plugin's instance
/// carries.
fn a_plugin_instance(match_name: &str) -> EffectInstance {
    let mut inst = instantiate(match_name).expect("the catalogue knows it");
    assert_eq!(
        inst.effect.namespace,
        crate::model::EffectNamespace::Ofx,
        "an ofx: name instantiates in the plugin namespace"
    );
    inst.enabled = true;
    inst
}

/// The whole point of the widening: an effect nobody compiled in is found by
/// the same lookup a built-in is found by, and the built-in menu order is
/// untouched by its arrival.
#[test]
fn a_registered_definition_joins_the_catalogue_behind_the_builtins() {
    let before: Vec<&str> = BUILTIN_DEFS.iter().map(|d| d.schema().match_name).collect();
    let builtin_count = BUILTINS.len();

    let schema = a_registered_schema("ofx:test.core.joins", &[0]);
    let def: &'static RegisteredDef = Box::leak(Box::new(RegisteredDef {
        schema,
        window: None,
    }));
    assert!(BUILTIN_DEFS.register(def), "it registered");

    // Found by name, and it is the definition that was registered.
    let found = BUILTIN_DEFS
        .get("ofx:test.core.joins")
        .expect("the catalogue answers to it");
    assert_eq!(found.schema().match_name, "ofx:test.core.joins");
    assert!(std::ptr::eq(found.schema(), schema));
    assert!(crate::fx::builtins::schema("ofx:test.core.joins").is_some());

    // The built-ins are still the first `BUILTINS.len()` entries, in their own
    // order: the Add-effect menu, the command palette and the preset browser all
    // read that order, and a plugin must never move it.
    let after: Vec<&str> = BUILTIN_DEFS.iter().map(|d| d.schema().match_name).collect();
    assert_eq!(
        &after[..builtin_count],
        &BUILTINS.iter().map(|s| s.match_name).collect::<Vec<_>>()[..],
        "a plugin reordered the built-in menu"
    );
    assert_eq!(&after[..builtin_count], &before[..builtin_count]);
    assert!(after.contains(&"ofx:test.core.joins"));
    assert!(BUILTIN_DEFS.len() > builtin_count);

    // A second registration under the same name is a rescan, not a second
    // effect: nothing is added, and the first definition still answers.
    let twin: &'static RegisteredDef = Box::leak(Box::new(RegisteredDef {
        schema: a_registered_schema("ofx:test.core.joins", &[0]),
        window: None,
    }));
    assert!(!BUILTIN_DEFS.register(twin), "a duplicate name is refused");
    let still = BUILTIN_DEFS
        .get("ofx:test.core.joins")
        .expect("still there");
    assert!(std::ptr::eq(still.schema(), schema));

    // And a name the built-ins already own is refused too, whichever way round.
    let shadow: &'static RegisteredDef = Box::leak(Box::new(RegisteredDef {
        schema: a_registered_schema("blur", &[0]),
        window: None,
    }));
    assert!(
        !BUILTIN_DEFS.register(shadow),
        "a built-in cannot be shadowed"
    );
    assert_eq!(
        crate::fx::builtins::schema("blur").map(|s| s.match_name),
        Some("blur")
    );
}

/// A retimer's declared frames reach the neighbour window, and therefore the
/// frame key and the prefetch (docs/12 §2.1). The declaration on the
/// schema is the fallback; the instance's own answer wins.
#[test]
fn a_retimers_declared_frames_land_in_the_temporal_window() {
    // Declared ±1 at describe time — the widening the host applies when a plugin
    // says it reads other frames at all.
    let schema = a_registered_schema("ofx:test.core.retimer", &[-1, 0, 1]);
    let def: &'static RegisteredDef = Box::leak(Box::new(RegisteredDef {
        schema,
        window: Some(vec![-5, -4, -3, -2, -1, 0, 1, 2, 3, 4, 5]),
    }));
    assert!(BUILTIN_DEFS.register(def));

    let inst = a_plugin_instance("ofx:test.core.retimer");
    let one = std::slice::from_ref(&inst);
    assert_eq!(
        stack_temporal_window(one, true, 0.0, 30.0),
        vec![-5, -4, -3, -2, -1, 0, 1, 2, 3, 4, 5],
        "the plugin's own frames are what the layer decodes"
    );
    // The gate in front of it is the declaration, which is why the widening at
    // describe time matters.
    assert!(stack_is_temporal(one, true));

    // Bypassed, disabled, and a plain built-in stack are all untouched.
    assert_eq!(stack_temporal_window(one, false, 0.0, 30.0), vec![0]);
    let mut off = inst.clone();
    off.enabled = false;
    assert_eq!(
        stack_temporal_window(std::slice::from_ref(&off), true, 0.0, 30.0),
        vec![0]
    );

    // A registered effect that answers `None` falls back to its declaration,
    // exactly as every built-in does.
    let plain_schema = a_registered_schema("ofx:test.core.plain", &[-1, 0, 1]);
    let plain: &'static RegisteredDef = Box::leak(Box::new(RegisteredDef {
        schema: plain_schema,
        window: None,
    }));
    assert!(BUILTIN_DEFS.register(plain));
    let plain_inst = a_plugin_instance("ofx:test.core.plain");
    assert_eq!(
        stack_temporal_window(std::slice::from_ref(&plain_inst), true, 0.0, 30.0),
        vec![-1, 0, 1]
    );
}

/// A plugin instance resolves into the arena beside the built-ins, in stack
/// order, carrying the layer time its parameters were read at.
#[test]
fn a_plugin_instance_resolves_between_two_builtins() {
    let schema = a_registered_schema("ofx:test.core.stack", &[0]);
    let def: &'static RegisteredDef = Box::leak(Box::new(RegisteredDef {
        schema,
        window: None,
    }));
    assert!(BUILTIN_DEFS.register(def));

    let stack = vec![
        instantiate("invert").expect("a built-in"),
        a_plugin_instance("ofx:test.core.stack"),
        instantiate("exposure").expect("a built-in"),
    ];
    let resolved = super::resolved::resolve_stack(
        &stack,
        1.5,
        1000.0,
        1.0,
        &MarkerContext::NONE,
        Arc::new(ExpressionContext::detached()),
    );
    let names: Vec<&str> = resolved
        .iter()
        .map(|op| op.def.schema().match_name)
        .collect();
    assert_eq!(names, vec!["invert", "ofx:test.core.stack", "exposure"]);
    let plugin_op = resolved.get(1).expect("the middle op");
    assert!(
        (plugin_op.lt - 1.5).abs() < 1e-9,
        "the op carries the layer time its values were read at"
    );
}
