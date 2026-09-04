//! `#[derive(Effect)]` — one declaration per built-in effect.
//!
//! **In plain terms.** An effect used to be written down five times in five
//! files. This macro is what makes one of those five the source of the rest: you
//! write a struct whose fields are the effect's controls, with the slider range
//! and the default written on each field, and the macro produces the catalogue
//! entry, the compile-time parameter ids, and the reader that pulls the controls
//! back out of a rendered frame's numbers.
//!
//! The generated code is deliberately boring — a `const` struct literal and a
//! field-by-field constructor — so that what an effect *is* stays readable in the
//! expansion. See `docs/impl/effect-registry.md` §2.1 for the shape and the full
//! attribute table.
//!
//! Four parameters are **injected** rather than declared: the Matte layer row,
//! its Invert switch and its Channel choice, which every effect
//! gets so the row means something on all of them from day one, and the Blend
//! choice beside every Mix slider. `matte_channel = false` keeps the
//! Channel off an effect that picks its matte's channels itself. `matte = "<id>"` says the effect
//! reads the matte out of the named parameter *itself*, inside its own maths,
//! instead of the generic dissolve — `matte = "matte"` for an effect that takes
//! the injected row and means something deeper by it (Gaussian blur, Glow), and
//! the effect's own older id for one that owned the idea first (Depth of
//! field's `depth`, the Lens flare's `matte`, which it declares itself and so
//! is not injected twice).
//!
//! ```ignore
//! #[derive(Effect)]
//! #[effect(
//!     match_name = "saturation",
//!     label = "Saturation",
//!     version = 1,
//!     category = Colour,
//!     cost = Trivial,
//!     roi = Exact,
//! )]
//! pub struct Saturation {
//!     #[slider(min = -100.0, max = 100.0, default = 0.0, hard_min = -100.0)]
//!     pub amount: f32,
//! }
//! ```

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    parse_macro_input, spanned::Spanned, Data, DeriveInput, Expr, ExprLit, Fields, Lit, Meta, Token,
};

/// Derive an effect's declaration from its parameter struct.
///
/// Generates `impl EffectMetadata`, one `ParamId` associated const per field, and
/// an `impl EffectDef` carrying the generated schema. The effect writes its own
/// `apply_cpu` in a separate `impl` block if it has an image operation.
#[proc_macro_derive(
    Effect,
    attributes(
        effect, slider, bounded, counter, dial, toggle, choice, colour, seed, file, layer,
        mask_path, curve, action
    )
)]
pub fn derive_effect(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// The struct-level `#[effect(...)]` attribute, parsed.
struct EffectAttr {
    match_name: String,
    label: String,
    version: TokenStream2,
    category: TokenStream2,
    cost: TokenStream2,
    roi: TokenStream2,
    temporal: TokenStream2,
    premultiplied: TokenStream2,
    seeded: TokenStream2,
    beat_input: TokenStream2,
    groups: TokenStream2,
    enabled_when: TokenStream2,
    /// What this effect's Matte row means. Default [`MatteAttr::Strength`].
    matte: MatteAttr,
    /// Whether the injected Matte row also gets the Channel choice.
    /// `matte_channel = false` for an effect that owns a channel choice for
    /// its matte already (Set matte, Displacement map, Depth of field, the
    /// Lens flare). Default true.
    matte_channel: bool,
}

/// The generic Matte parameter's id, repeated here because a proc-macro crate
/// cannot depend on `lumit-core`. `lumit_core::fx::MATTE_PARAM` is the
/// definition — the emitted schema uses *that* const for the id, and this copy
/// only decides whether to emit at all. If the two ever drift, the effect either
/// gains no row or gains a second one under the same id, and
/// `every_effect_carries_a_matte_row` in lumit-core fails on the first effect it
/// walks.
const MATTE_PARAM: &str = "matte";

/// The Mix slider's id, the Blend row's and the Channel row's — the
/// same copies-for-the-same-reason as [`MATTE_PARAM`]: the emitted schema uses
/// the `lumit_core::fx` consts for the ids, and these only decide whether to
/// emit.
const MIX_PARAM: &str = "mix";
const BLEND_PARAM: &str = "blend";

/// The `matte = ...` attribute's three spellings, which become
/// `lumit_core::fx::MatteRole` in the emitted schema.
enum MatteAttr {
    /// `matte = false` — no row, no slot, no dissolve.
    None,
    /// Absent, or `matte = true` — the generic strength dissolve.
    Strength,
    /// `matte = ("<param id>", "<what it means>")` — the effect consumes the
    /// matte inside its own maths, out of the named parameter. `"matte"` is the
    /// injected row read by the kernel instead of the dissolve (Gaussian blur,
    /// Glow); any other id is a row the effect already declares itself (Depth of
    /// field's `depth`). The sentence rides in the same attribute because an
    /// override must document its meaning, and a separate optional attribute is
    /// one an override can forget.
    Own(String, String),
}

impl MatteAttr {
    /// The parameter id the matte layer is stored under, if any.
    fn role(&self) -> Option<&str> {
        match self {
            MatteAttr::None => None,
            MatteAttr::Strength => Some(MATTE_PARAM),
            MatteAttr::Own(id, _) => Some(id),
        }
    }

    fn tokens(&self) -> TokenStream2 {
        match self {
            MatteAttr::None => quote! { ::lumit_core::fx::MatteRole::None },
            MatteAttr::Strength => quote! { ::lumit_core::fx::MatteRole::Strength },
            MatteAttr::Own(id, meaning) => quote! {
                ::lumit_core::fx::MatteRole::Own { param: #id, meaning: #meaning }
            },
        }
    }
}

fn expand(input: DeriveInput) -> syn::Result<TokenStream2> {
    let ty = &input.ident;
    let effect = parse_effect_attr(&input)?;

    let fields = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(f) => f.named.iter().collect::<Vec<_>>(),
            Fields::Unit => Vec::new(),
            _ => {
                return Err(syn::Error::new(
                    input.span(),
                    "an effect's parameters must be named fields",
                ))
            }
        },
        _ => {
            return Err(syn::Error::new(
                input.span(),
                "#[derive(Effect)] applies to a struct of parameters",
            ))
        }
    };

    let mut schemas = Vec::new();
    let mut id_consts = Vec::new();
    let mut readers = Vec::new();
    // Every id the struct declares, so the Matte injection below can tell "this
    // effect wants the generic row" from "this effect already has one".
    let mut declared: Vec<String> = Vec::new();

    for field in fields {
        let Some(name) = field.ident.clone() else {
            continue;
        };
        let param = parse_param(field, &name)?;
        declared.push(param.id.clone());
        let id = &param.id;
        let label = &param.label;
        let kind = &param.kind;
        let unit = &param.unit;
        schemas.push(quote! {
            ::lumit_core::fx::ParamSchema {
                id: #id,
                label: #label,
                kind: #kind,
                unit: #unit,
            }
        });
        let const_name = format_ident!("{}", name.to_string().to_uppercase());
        id_consts.push(quote! {
            /// The compile-time id of this effect's parameter of the same name.
            pub const #const_name: ::lumit_core::fx::ParamId =
                ::lumit_core::fx::ParamId::new(#id);
        });
        let read = &param.read;
        readers.push(quote! { #name: #read });
    }

    let EffectAttr {
        match_name,
        label,
        version,
        category,
        cost,
        roi,
        temporal,
        premultiplied,
        seeded,
        beat_input,
        groups,
        enabled_when,
        matte,
        matte_channel,
    } = effect;

    // The Blend row, injected right after the Mix slider on every
    // effect that has one and does not declare a `blend` of its own (the Lens
    // flare). An effect with no Mix touches no pixel (the Controls, the Camera
    // track, Posterize time) and has nothing to blend. It sits beside `mix` in
    // schema order because the panel draws it on the Mix row.
    if let Some(at) = declared.iter().position(|d| d == MIX_PARAM) {
        if !declared.iter().any(|d| d == BLEND_PARAM) {
            schemas.insert(
                at + 1,
                quote! {
                    ::lumit_core::fx::ParamSchema {
                        id: ::lumit_core::fx::BLEND_PARAM,
                        label: "Blend",
                        kind: ::lumit_core::fx::ParamKind::Choice {
                            options: ::lumit_core::model::BlendMode::NAMES,
                            default: 0,
                            dividers_after: ::lumit_core::fx::CHOICE_UNGROUPED,
                        },
                        unit: ::lumit_core::fx::Unit::Raw,
                    }
                },
            );
        }
    }

    // The Matte pair, appended to every effect's parameters unless the
    // declaration opted out. It is injected here rather than written down 33
    // times for the reason the unit table is: a declaration that has to repeat
    // something is a declaration that will one day forget it, and an effect
    // whose Matte row is missing looks exactly like an effect whose matte does
    // not work. There is no field to read back, so `read()` is untouched — the
    // layer binding rides beside the op and the switch is read out of
    // the bag by whoever consumes the matte, not by `read()`.
    //
    // **Injected only where it is wanted and missing.** An effect that claims
    // the matte inside its own maths still takes the generic row (Gaussian blur
    // and Glow do), so the test is the role's parameter *id*, not whether the
    // meaning is generic. An effect that owned the idea first already declares
    // the row itself — the Lens flare's `matte`, Depth of field's `depth` —
    // and injecting a second one would give it two rows with one id.
    let matte_role = matte.tokens();
    let inject = matte.role() == Some(MATTE_PARAM) && !declared.iter().any(|d| d == MATTE_PARAM);
    if inject {
        schemas.push(quote! {
            ::lumit_core::fx::ParamSchema {
                id: ::lumit_core::fx::MATTE_PARAM,
                label: "Matte",
                kind: ::lumit_core::fx::ParamKind::Layer { self_default: false },
                unit: ::lumit_core::fx::Unit::Raw,
            }
        });
        schemas.push(quote! {
            ::lumit_core::fx::ParamSchema {
                id: ::lumit_core::fx::MATTE_INVERT_PARAM,
                label: "Invert",
                kind: ::lumit_core::fx::ParamKind::Bool { default: false },
                unit: ::lumit_core::fx::Unit::Raw,
            }
        });
        // The Channel choice, unless the effect picks its matte's
        // channels itself.
        if matte_channel {
            schemas.push(quote! {
                ::lumit_core::fx::ParamSchema {
                    id: ::lumit_core::fx::MATTE_CHANNEL_PARAM,
                    label: "Channel",
                    kind: ::lumit_core::fx::ParamKind::Choice {
                        options: ::lumit_core::fx::CHANNEL_OPTIONS,
                        default: 0,
                        dividers_after: ::lumit_core::fx::CHOICE_UNGROUPED,
                    },
                    unit: ::lumit_core::fx::Unit::Raw,
                }
            });
        }
    }

    let doc =
        format!("The `{match_name}` effect's declaration, generated from its parameter struct.");

    Ok(quote! {
        impl #ty {
            #(#id_consts)*
        }

        impl ::lumit_core::fx::EffectMetadata for #ty {
            #[doc = #doc]
            const SCHEMA: ::lumit_core::fx::EffectSchema = ::lumit_core::fx::EffectSchema {
                match_name: #match_name,
                label: #label,
                version: #version,
                category: ::lumit_core::fx::FxCategory::#category,
                traits: ::lumit_core::fx::EffectTraits {
                    cost: ::lumit_core::fx::CostClass::#cost,
                    roi: ::lumit_core::fx::Roi::#roi,
                    temporal: #temporal,
                    premultiplied: #premultiplied,
                    seeded: #seeded,
                    beat_input: #beat_input,
                },
                params: &[#(#schemas),*],
                groups: #groups,
                enabled_when: #enabled_when,
                matte: #matte_role,
            };

            fn read(p: ::lumit_core::fx::Params<'_>) -> Self {
                let _ = &p;
                Self { #(#readers),* }
            }
        }
    })
}

fn parse_effect_attr(input: &DeriveInput) -> syn::Result<EffectAttr> {
    let attr = input
        .attrs
        .iter()
        .find(|a| a.path().is_ident("effect"))
        .ok_or_else(|| {
            syn::Error::new(
                input.span(),
                "an effect needs #[effect(match_name = \"...\", label = \"...\", version = 1, \
                 category = ..., cost = ..., roi = ...)]",
            )
        })?;

    let metas =
        attr.parse_args_with(syn::punctuated::Punctuated::<Meta, Token![,]>::parse_terminated)?;

    let mut match_name = None;
    let mut label = None;
    let mut version = None;
    let mut category = None;
    let mut cost = None;
    let mut roi = None;
    let mut temporal = quote! { &[0] };
    let mut premultiplied = quote! { true };
    let mut seeded = quote! { false };
    let mut beat_input = quote! { false };
    let mut groups = quote! { &[] };
    let mut enabled_when = quote! { &[] };
    let mut matte = MatteAttr::Strength;
    let mut matte_channel = true;

    for meta in metas {
        let Meta::NameValue(nv) = meta else {
            return Err(syn::Error::new(meta.span(), "expected `name = value`"));
        };
        let value = &nv.value;
        let key = nv
            .path
            .get_ident()
            .map(|i| i.to_string())
            .unwrap_or_default();
        match key.as_str() {
            "match_name" => match_name = Some(lit_str(value)?),
            "label" => label = Some(lit_str(value)?),
            "version" => version = Some(quote! { #value }),
            "category" => category = Some(quote! { #value }),
            "cost" => cost = Some(quote! { #value }),
            "roi" => roi = Some(quote! { #value }),
            "temporal" => temporal = quote! { #value },
            "premultiplied" => premultiplied = quote! { #value },
            "seeded" => seeded = quote! { #value },
            "beat_input" => beat_input = quote! { #value },
            "groups" => groups = quote! { #value },
            "enabled_when" => enabled_when = quote! { #value },
            // What the Matte row means. `false` opts out entirely; a
            // string names the parameter the effect reads the matte out of
            // *itself*, which is how an effect declares a deeper meaning than
            // strength — and how the two that owned the idea first keep their
            // stored ids.
            // Whether the injected matte row gets its Channel choice.
            "matte_channel" => {
                matte_channel = match value {
                    Expr::Lit(ExprLit {
                        lit: Lit::Bool(b), ..
                    }) => b.value(),
                    other => {
                        return Err(syn::Error::new(
                            other.span(),
                            "expected `matte_channel = false` for an effect that picks its                              matte's channel itself",
                        ))
                    }
                }
            }
            "matte" => {
                matte = match value {
                    Expr::Lit(ExprLit {
                        lit: Lit::Bool(b), ..
                    }) => {
                        if b.value() {
                            MatteAttr::Strength
                        } else {
                            MatteAttr::None
                        }
                    }
                    Expr::Tuple(t) if t.elems.len() == 2 => {
                        let mut it = t.elems.iter();
                        match (it.next(), it.next()) {
                            (Some(id), Some(meaning)) => {
                                MatteAttr::Own(lit_str(id)?, lit_str(meaning)?)
                            }
                            _ => unreachable!("len == 2 was just matched"),
                        }
                    }
                    other => {
                        return Err(syn::Error::new(
                            other.span(),
                            "expected `false` (no matte), `true` (the generic strength \
                             dissolve), or `(\"<param id>\", \"<what this effect's matte \
                             means>\")` — an override must say what it means",
                        ))
                    }
                }
            }
            other => {
                return Err(syn::Error::new(
                    nv.path.span(),
                    format!("unknown effect attribute `{other}`"),
                ))
            }
        }
    }

    let missing = |what: &str| syn::Error::new(attr.span(), format!("an effect needs `{what}`"));
    Ok(EffectAttr {
        match_name: match_name.ok_or_else(|| missing("match_name"))?,
        label: label.ok_or_else(|| missing("label"))?,
        version: version.ok_or_else(|| missing("version"))?,
        category: category.ok_or_else(|| missing("category"))?,
        cost: cost.ok_or_else(|| missing("cost"))?,
        roi: roi.ok_or_else(|| missing("roi"))?,
        temporal,
        premultiplied,
        seeded,
        beat_input,
        groups,
        enabled_when,
        matte,
        matte_channel,
    })
}

/// One parsed parameter: what goes in the schema, and how it is read back.
struct Param {
    id: String,
    label: String,
    kind: TokenStream2,
    unit: TokenStream2,
    read: TokenStream2,
}

fn parse_param(field: &syn::Field, name: &syn::Ident) -> syn::Result<Param> {
    let known = [
        "slider",
        "bounded",
        "counter",
        "dial",
        "toggle",
        "choice",
        "colour",
        "seed",
        "file",
        "layer",
        "mask_path",
        "curve",
        "action",
    ];
    let attr = field
        .attrs
        .iter()
        .find(|a| known.iter().any(|k| a.path().is_ident(k)))
        .ok_or_else(|| {
            syn::Error::new(
                field.span(),
                "every field is a parameter and needs one of #[slider] #[bounded] #[counter] \
                 #[dial] #[toggle] #[choice] #[colour] #[seed] #[file] #[layer] #[mask_path] \
                 #[curve] #[action]",
            )
        })?;

    let which = attr
        .path()
        .get_ident()
        .map(|i| i.to_string())
        .unwrap_or_default();

    // `#[seed]` and `#[layer]` carry no arguments in the common case.
    let args: Vec<Meta> = match &attr.meta {
        Meta::Path(_) => Vec::new(),
        _ => attr
            .parse_args_with(syn::punctuated::Punctuated::<Meta, Token![,]>::parse_terminated)?
            .into_iter()
            .collect(),
    };

    let get = |key: &str| -> Option<TokenStream2> {
        args.iter().find_map(|m| match m {
            Meta::NameValue(nv) if nv.path.is_ident(key) => {
                let v = &nv.value;
                Some(quote! { #v })
            }
            _ => None,
        })
    };

    let id = get("id")
        .and_then(|_| {
            args.iter().find_map(|m| match m {
                Meta::NameValue(nv) if nv.path.is_ident("id") => lit_str(&nv.value).ok(),
                _ => None,
            })
        })
        .unwrap_or_else(|| name.to_string());
    let label = args
        .iter()
        .find_map(|m| match m {
            Meta::NameValue(nv) if nv.path.is_ident("label") => lit_str(&nv.value).ok(),
            _ => None,
        })
        .unwrap_or_else(|| sentence_case(&id));

    let idc = quote! { ::lumit_core::fx::ParamId::new(#id) };
    // What the number means (the unit rider, and the resolve step's spatial
    // rescale). A `#[dial]` is degrees by definition and a control that carries
    // no number carries no unit, so those two answer themselves; a slider, a
    // bounded slider or a counter has a genuine choice to make, and saying
    // nothing is `Unset` — which the catalogue test fails the build on, rather
    // than the silent `Raw` that used to make "dimensionless" and "nobody
    // decided" the same declaration.
    let unit = match get("unit") {
        Some(u) => quote! { ::lumit_core::fx::Unit::#u },
        None => match which.as_str() {
            "slider" | "bounded" | "counter" => quote! { ::lumit_core::fx::Unit::Unset },
            "dial" => quote! { ::lumit_core::fx::Unit::Degrees },
            _ => quote! { ::lumit_core::fx::Unit::Raw },
        },
    };

    let (kind, read) = match which.as_str() {
        "slider" => {
            let default = get("default").unwrap_or_else(|| quote! { 0.0 });
            let min = get("min").unwrap_or_else(|| quote! { 0.0 });
            let max = get("max").unwrap_or_else(|| quote! { 1.0 });
            let hard_min = opt(get("hard_min"));
            let hard_max = opt(get("hard_max"));
            (
                quote! {
                    ::lumit_core::fx::ParamKind::Float {
                        default: #default,
                        slider: (#min, #max),
                        hard: (#hard_min, #hard_max),
                    }
                },
                quote! { p.float(#idc, (#default) as f32) },
            )
        }
        // A closed range, drawn as a track and thumb. The one
        // difference from `#[slider]` is that there is no soft/hard pair to
        // declare: the range *is* both, because a parameter whose whole
        // meaning lives inside it has nothing to mean outside it.
        "bounded" => {
            let default = get("default").unwrap_or_else(|| quote! { 0.0 });
            let min = get("min").unwrap_or_else(|| quote! { 0.0 });
            let max = get("max").unwrap_or_else(|| quote! { 1.0 });
            (
                quote! {
                    ::lumit_core::fx::ParamKind::Slider {
                        default: #default,
                        range: (#min, #max),
                    }
                },
                quote! { p.float(#idc, (#default) as f32) },
            )
        }
        "counter" => {
            let default = get("default").unwrap_or_else(|| quote! { 0 });
            let min = get("min").unwrap_or_else(|| quote! { 0 });
            let max = get("max").unwrap_or_else(|| quote! { 10 });
            let hard_min = opt(get("hard_min"));
            let hard_max = opt(get("hard_max"));
            (
                quote! {
                    ::lumit_core::fx::ParamKind::Int {
                        default: #default,
                        slider: (#min, #max),
                        hard: (#hard_min, #hard_max),
                    }
                },
                quote! { p.int(#idc, (#default) as i32) },
            )
        }
        "dial" => {
            let default = get("default").unwrap_or_else(|| quote! { 0.0 });
            let step = get("step").unwrap_or_else(|| quote! { 15.0 });
            (
                quote! {
                    ::lumit_core::fx::ParamKind::Angle {
                        default: #default,
                        dial_step: #step,
                    }
                },
                quote! { p.float(#idc, (#default) as f32) },
            )
        }
        "toggle" => {
            let default = get("default").unwrap_or_else(|| quote! { false });
            (
                quote! { ::lumit_core::fx::ParamKind::Bool { default: #default } },
                quote! { p.bool(#idc, #default) },
            )
        }
        "choice" => {
            let options = get("options").ok_or_else(|| {
                syn::Error::new(attr.span(), "a #[choice] needs `options = [\"…\", \"…\"]`")
            })?;
            let default = get("default").unwrap_or_else(|| quote! { 0 });
            let dividers = get("dividers_after").unwrap_or_else(|| quote! { &[] });
            (
                quote! {
                    ::lumit_core::fx::ParamKind::Choice {
                        options: &#options,
                        default: #default,
                        dividers_after: #dividers,
                    }
                },
                quote! { p.choice(#idc, #default) },
            )
        }
        "colour" => {
            let default = get("default").unwrap_or_else(|| quote! { [1.0, 1.0, 1.0, 1.0] });
            let min = get("min").unwrap_or_else(|| quote! { 0.0 });
            let max = get("max").unwrap_or_else(|| quote! { 1.0 });
            (
                quote! {
                    ::lumit_core::fx::ParamKind::Colour {
                        default: #default,
                        range: (#min, #max),
                    }
                },
                quote! {{
                    const D: [f64; 4] = #default;
                    p.colour(#idc, [D[0] as f32, D[1] as f32, D[2] as f32, D[3] as f32])
                }},
            )
        }
        "seed" => (
            quote! { ::lumit_core::fx::ParamKind::Seed },
            quote! { p.int(#idc, 0) as u32 },
        ),
        "file" => {
            let filter = get("filter").unwrap_or_else(|| quote! { [] });
            let filter_name = get("filter_name").unwrap_or_else(|| quote! { "File" });
            (
                quote! {
                    ::lumit_core::fx::ParamKind::File {
                        filter: &#filter,
                        filter_name: #filter_name,
                    }
                },
                quote! { p.file_slot(#idc) },
            )
        }
        "layer" => {
            let self_default = get("self_default").unwrap_or_else(|| quote! { false });
            (
                quote! { ::lumit_core::fx::ParamKind::Layer { self_default: #self_default } },
                quote! { p.layer_bound(#idc) },
            )
        }
        // One of this layer's masks, handed to the effect as geometry.
        // `self_default` mirrors `#[layer]`'s and defaults the other way round:
        // an unset row means the layer's **first mask**, because an effect that
        // wants a path wants the one path most layers have, and an effect is
        // usually dropped on before the mask is drawn.
        "mask_path" => {
            let self_default = get("self_default").unwrap_or_else(|| quote! { true });
            (
                quote! { ::lumit_core::fx::ParamKind::MaskPath { self_default: #self_default } },
                quote! { p.mask_named(#idc) },
            )
        }
        // A tone curve, as its own control points. There is no range to
        // declare — the points live in the unit square by definition — and the
        // default is the identity diagonal unless the declaration gives a shape
        // of its own, which only an over-life curve does (particulate.md §2).
        "curve" => {
            let default =
                get("default").unwrap_or_else(|| quote! { ::lumit_core::fx::CURVE_IDENTITY });
            (
                quote! { ::lumit_core::fx::ParamKind::Curve { default: &#default } },
                quote! { p.curve(#idc) },
            )
        }
        // A button. It carries no value, so the field it declares is
        // the unit type and `read` fills it with `()`: there is nothing in the
        // bag to read back, and there never will be — an Action crosses the
        // bridge as an event. The generated `ParamId` const is still emitted,
        // which is what the event names the row by. `#[action(label = "…")]`
        // is the only argument it takes, and it comes from the shared label
        // handling above like every other kind's.
        "action" => (
            quote! { ::lumit_core::fx::ParamKind::Action },
            quote! { () },
        ),
        other => {
            return Err(syn::Error::new(
                attr.span(),
                format!("unknown parameter kind `{other}`"),
            ))
        }
    };

    Ok(Param {
        id,
        label,
        kind,
        unit,
        read,
    })
}

/// `Some(x)` / `None`, for the one-sided hard bounds.
fn opt(v: Option<TokenStream2>) -> TokenStream2 {
    match v {
        Some(v) => quote! { Some(#v) },
        None => quote! { None },
    }
}

fn lit_str(e: &Expr) -> syn::Result<String> {
    match e {
        Expr::Lit(ExprLit {
            lit: Lit::Str(s), ..
        }) => Ok(s.value()),
        other => Err(syn::Error::new(other.span(), "expected a string literal")),
    }
}

/// `near_aperture` → `Near aperture`: the UI label a parameter gets when it does
/// not declare one. Sentence case, per docs/15-DESIGN.md.
fn sentence_case(id: &str) -> String {
    let spaced = id.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => spaced,
    }
}
