//! The Lumit theme: a rerun-inspired dark system (K-084), per
//! docs/15-DESIGN.md — Lumit's own hues on rerun.io's structure: a
//! near-black canvas, panels barely above it, floating surfaces a clear step
//! up, borderless widgets whose states are fill changes, crisp 1 px hairlines,
//! and thin solid scrollbars. K-092 adds two independent axes on top: a
//! light ramp alongside the dark one (`ThemeMode`), and a second panel
//! geometry — rounded floating cards instead of edge-to-edge rectangles
//! (`ThemeShape`, carried as `ShapeTokens`).
//!
//! This module is the ONLY place in the codebase where colour hex values may
//! appear (enforced by CI grep). Everything else reads the `Theme` struct.
//! Font embedding (Inter per the design doc) is a follow-up pending the owner's
//! nod on shipping the font file; until then egui's defaults stand in.

use egui::{Color32, CornerRadius, Shadow, Stroke, Visuals};

/// Semantic colour tokens. Names mirror docs/15-DESIGN.md §tokens.
#[derive(Clone, Copy)]
pub struct Theme {
    /// Light or dark colour family (K-092) — needed here (not just at
    /// construction) because `with_accent`'s hover-shift direction depends
    /// on it: brightening reads as "more prominent" on a dark surface,
    /// darkening on a light one.
    pub mode: ThemeMode,
    /// Sharp (edge-to-edge, hairline) or Round (floating card) geometry
    /// (K-092) — read by `DockBehavior`/`pane_ui` in shell.rs to decide
    /// whether to wrap a pane's content in a rounded, shadowed card.
    pub shape: ThemeShape,
    /// The geometry numbers `shape` selects — radii, gap, shadow. See
    /// [`ShapeTokens`].
    pub tokens: ShapeTokens,

    // Surfaces (near-neutral ramp; direction depends on `mode`)
    pub surface_0: Color32,
    pub surface_1: Color32,
    pub surface_2: Color32,
    pub surface_3: Color32,
    pub surface_4: Color32,
    /// The Viewer pasteboard — exactly neutral, R = G = B (grading accuracy).
    /// Deliberately NOT mode-mirrored: a fixed mid-grey neighbourhood in
    /// both Dark and Light, since its whole purpose is staying decoupled
    /// from chrome brightness for grading judgement (15-DESIGN §2.1/§11).
    pub viewer_surround: Color32,

    // Text
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_muted: Color32,
    pub text_disabled: Color32,

    // Hairlines (1 px borders carry elevation; shadows only for true floats)
    pub hairline: Color32,
    pub hairline_strong: Color32,

    // Roles — clay is THE single accent per view
    pub accent: Color32,
    pub accent_hover: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub error: Color32,
    /// The cache bar's disk tier (docs/06 §5.6 "blue — on disk, promotable");
    /// calm steel blue, quieter than the RAM tier's mint.
    pub cache_disk: Color32,
    /// Graph-editor curve strokes (15-DESIGN §graph: the viz ramp).
    pub curve: [Color32; 4],
    /// Layer-type identity colours (15-DESIGN §6.1).
    pub layer: LayerColours,
}

/// Light vs dark colour family (K-092). Orthogonal to [`ThemeShape`] and to
/// [`ThemeVariant`] (which only means something under `Dark` — it picks
/// *which* dark ramp; there is one light ramp).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

/// Which panel geometry the chrome uses (K-092, owner request). Sharp is
/// the existing edge-to-edge hairline system (unchanged pixel-for-pixel);
/// Round is the Figma-UI3-inspired floating-card system: rounded corners,
/// visible gaps between panels and from the window edge, soft shadow
/// standing in for the hairline as the elevation cue. Orthogonal to
/// `ThemeMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ThemeShape {
    #[default]
    Sharp,
    Round,
}

/// Shape-dependent chrome geometry (K-092): every number that changes
/// between Sharp and Round but carries no colour of its own — colours stay
/// plain `Theme` fields, shared by both shapes. `SHARP` reproduces the
/// pre-K-092 hardcoded numbers exactly (a regression test pins this), so
/// picking Sharp is a byte-for-byte no-op.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeTokens {
    /// Buttons, inputs, chips.
    pub control_radius: u8,
    /// Menus, popups, windows/dialogs.
    pub float_radius: u8,
    /// A docked pane's own card (0 under Sharp — no card at all).
    pub card_radius: u8,
    /// Inner margin inside a pane's card, between its rounded edge and its
    /// content (0 under Sharp). Kept comfortably above the geometric
    /// minimum that would let a content rect's square corner poke past the
    /// card's rounded silhouette (inset ≥ radius × (1 − 1/√2)).
    pub card_padding: i8,
    /// egui_tiles' inter-pane gap (existing behaviour is `1.0` — a hairline
    /// divider; Round widens it so the canvas shows through).
    pub tile_gap: f32,
    /// The dock's inset from the OS window edge (0 under Sharp).
    pub window_inset: f32,
    /// A docked pane's own shadow (`Shadow::NONE` under Sharp — ordinary
    /// panels don't float there; a small soft shadow under Round).
    pub card_shadow: Shadow,
}

impl ShapeTokens {
    /// Today's edge-to-edge system: zero gap, zero inset, no card, no
    /// per-pane shadow, the existing 4px/6px radii.
    pub const SHARP: Self = Self {
        control_radius: 4,
        float_radius: 6,
        card_radius: 0,
        card_padding: 0,
        tile_gap: 1.0,
        window_inset: 0.0,
        card_shadow: Shadow::NONE,
    };

    /// The Figma-UI3-inspired floating-card system: a real gap between
    /// panels and from the window edge, rounded cards, a soft small shadow
    /// (offset/blur well under the float shadow's, so cards read as
    /// "gently elevated" rather than "floating menu").
    pub const ROUND: Self = Self {
        control_radius: 8,
        float_radius: 12,
        card_radius: 14,
        card_padding: 10,
        tile_gap: 12.0,
        window_inset: 12.0,
        card_shadow: Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha(0x30),
        },
    };
}

/// Per-layer-type identity colours (docs/15-DESIGN.md §6.1): muted siblings,
/// every one clearly quieter than `accent` so a full Timeline reads as organised
/// rather than carnival. Each type carries its colour as a left-edge tab and the
/// tint of its type glyph.
#[derive(Clone, Copy)]
pub struct LayerColours {
    pub footage: Color32,
    pub sequence: Color32,
    pub precomp: Color32,
    pub solid: Color32,
    pub text: Color32,
    pub camera: Color32,
}

impl Theme {
    pub const fn dark() -> Self {
        Self {
            mode: ThemeMode::Dark,
            shape: ThemeShape::Sharp,
            tokens: ShapeTokens::SHARP,
            // The rerun-structure ramp (K-084) in Lumit's cool grey: the
            // canvas sits near black, panels barely above it, "faint" surfaces
            // one step up, floating surfaces (menus, inputs, tab bars) a clear
            // step above that, and widget fills brightest of the ramp.
            surface_0: Color32::from_rgb(0x0b, 0x0c, 0x0e),
            surface_1: Color32::from_rgb(0x13, 0x15, 0x17),
            surface_2: Color32::from_rgb(0x1a, 0x1d, 0x20),
            surface_3: Color32::from_rgb(0x21, 0x25, 0x28),
            surface_4: Color32::from_rgb(0x2b, 0x30, 0x34),
            viewer_surround: Color32::from_rgb(0x12, 0x12, 0x12),

            text_primary: Color32::from_rgb(0xee, 0xf1, 0xf2),
            text_secondary: Color32::from_rgb(0xc2, 0xc8, 0xcb),
            text_muted: Color32::from_rgb(0x8b, 0x92, 0x96),
            text_disabled: Color32::from_rgb(0x5e, 0x66, 0x6b),

            // Crisp 1 px separations; strong doubles as the pressed widget fill.
            hairline: Color32::from_rgba_premultiplied(0x26, 0x29, 0x2c, 0xff),
            hairline_strong: Color32::from_rgba_premultiplied(0x3c, 0x41, 0x45, 0xff),

            accent: Color32::from_rgb(0xe0, 0x5a, 0x72),
            accent_hover: Color32::from_rgb(0xea, 0x72, 0x88),
            success: Color32::from_rgb(0x5f, 0xcf, 0xae),
            warning: Color32::from_rgb(0xdd, 0x9a, 0x82),
            error: Color32::from_rgb(0xd1, 0x72, 0x9c),
            cache_disk: Color32::from_rgb(0x5f, 0x93, 0xb8),
            curve: [
                Color32::from_rgb(0x8e, 0xe3, 0xef),
                Color32::from_rgb(0xae, 0xf3, 0xe7),
                Color32::from_rgb(0xe8, 0xa7, 0xb4),
                Color32::from_rgb(0xd8, 0xcb, 0xa0),
            ],
            layer: LayerColours {
                footage: Color32::from_rgb(0x56, 0x70, 0x7f),  // steel
                sequence: Color32::from_rgb(0x5a, 0x6a, 0x8c), // indigo
                precomp: Color32::from_rgb(0x7a, 0x5a, 0x74),  // plum
                solid: Color32::from_rgb(0x5c, 0x61, 0x65),    // neutral
                text: Color32::from_rgb(0x8c, 0x84, 0x68),     // parchment
                camera: Color32::from_rgb(0x80, 0x6f, 0x4a),   // dry gold
            },
        }
    }

    /// The light ramp (K-092): one uniform light theme — every panel the
    /// same plain white, no per-panel colour-tinting (the owner's explicit
    /// call; that idea is wanted later as its own customisable setting, not
    /// built here). Surfaces keep the same *roles* as `dark()`
    /// (`surface_1` = panel/active content, `surface_2` = faint/tab-bar
    /// chrome, `surface_3` = floating, `surface_4` = hover/pressed fill),
    /// but since white is already the brightest possible value, "elevation"
    /// on a light ramp reads as a light grey wash rather than a further
    /// brightening — the same idea `dark()` expresses as steps toward
    /// white, inverted at the point where there's no more headroom.
    /// `viewer_surround` is NOT a mirror of the dark value — see the field
    /// doc on `Theme::viewer_surround`.
    pub const fn light() -> Self {
        Self {
            mode: ThemeMode::Light,
            shape: ThemeShape::Sharp,
            tokens: ShapeTokens::SHARP,
            // Canvas recedes a little from the white panels it holds —
            // the same "canvas calmer than content" relationship dark()
            // has, just at the light end of the scale.
            surface_0: Color32::from_rgb(0xee, 0xec, 0xe9),
            surface_1: Color32::from_rgb(0xff, 0xff, 0xff),
            surface_2: Color32::from_rgb(0xf6, 0xf5, 0xf3),
            surface_3: Color32::from_rgb(0xff, 0xff, 0xff),
            surface_4: Color32::from_rgb(0xe9, 0xe7, 0xe4),
            viewer_surround: Color32::from_rgb(0xa8, 0xa8, 0xa8),

            text_primary: Color32::from_rgb(0x1a, 0x1a, 0x18),
            text_secondary: Color32::from_rgb(0x45, 0x45, 0x42),
            text_muted: Color32::from_rgb(0x7a, 0x7a, 0x76),
            text_disabled: Color32::from_rgb(0xa8, 0xa8, 0xa4),

            // Same "hairline as a slice of text_primary" rule as dark(),
            // re-run against the near-black anchor: a subtle dark line on
            // a light surface rather than a subtle light line on a dark one.
            hairline: Color32::from_rgba_premultiplied(0xd8, 0xd6, 0xd2, 0xff),
            hairline_strong: Color32::from_rgba_premultiplied(0xc4, 0xc1, 0xbc, 0xff),

            // Roles re-picked at reduced lightness for contrast on white,
            // not naive-inverted (a value as light as the dark-mode accent
            // would wash out here) — same clay/success/warning/error hues,
            // deeper.
            accent: Color32::from_rgb(0xc2, 0x3f, 0x58),
            accent_hover: Color32::from_rgb(0xa8, 0x30, 0x48),
            success: Color32::from_rgb(0x2f, 0x8f, 0x71),
            warning: Color32::from_rgb(0xb5, 0x5f, 0x46),
            error: Color32::from_rgb(0x9c, 0x3f, 0x66),
            cache_disk: Color32::from_rgb(0x2f, 0x5f, 0x82),
            curve: [
                Color32::from_rgb(0x2f, 0x8a, 0x96),
                Color32::from_rgb(0x3f, 0x9c, 0x8e),
                Color32::from_rgb(0xb5, 0x5f, 0x6e),
                Color32::from_rgb(0x8a, 0x76, 0x42),
            ],
            layer: LayerColours {
                footage: Color32::from_rgb(0x3d, 0x52, 0x60),
                sequence: Color32::from_rgb(0x40, 0x4d, 0x68),
                precomp: Color32::from_rgb(0x5c, 0x40, 0x56),
                solid: Color32::from_rgb(0x42, 0x46, 0x49),
                text: Color32::from_rgb(0x66, 0x5e, 0x46),
                camera: Color32::from_rgb(0x5e, 0x50, 0x30),
            },
        }
    }

    /// The full composition (K-092): mode × variant × shape. `variant` only
    /// has meaning under `ThemeMode::Dark` (there is one light ramp; a
    /// Dark/DarkBlue choice made while in Light mode is inert, matching
    /// what happens if it's ignored). The only entry point `Shell` and the
    /// Window menu need — `Theme::of`/`Theme::light` stay as the
    /// ramp-only building blocks this composes.
    pub const fn for_settings(mode: ThemeMode, variant: ThemeVariant, shape: ThemeShape) -> Self {
        let mut t = match mode {
            ThemeMode::Dark => Self::of(variant),
            ThemeMode::Light => Self::light(),
        };
        t.tokens = match shape {
            ThemeShape::Sharp => ShapeTokens::SHARP,
            ThemeShape::Round => ShapeTokens::ROUND,
        };
        t.shape = shape;
        t
    }

    /// Install the UI typeface: Inter Medium (SIL OFL 1.1 — free, commercial
    /// use included; `assets/fonts/LICENSE.txt`, shared with the text engine's
    /// Inter Regular). First in the proportional family, so every label takes
    /// it; egui's bundled faces stay behind it as glyph fallbacks.
    pub fn install_fonts(ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "Inter-Medium".to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
                "../../../assets/fonts/Inter-Medium.otf"
            ))),
        );
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "Inter-Medium".to_owned());
        // The Iconoir icon font joins the same definitions (K-085) — one
        // set_fonts call carries the whole type system.
        crate::icons::install(&mut fonts);
        ctx.set_fonts(fonts);
    }

    /// Apply the theme to an egui context: visuals, spacing, type scale — the
    /// rerun-inspired system (K-084). Idle widgets are borderless (hover and
    /// press carry an edge, owner amendment); menus float on a soft real
    /// shadow; scrollbars are thin and solid.
    pub fn apply(&self, ctx: &egui::Context) {
        let mut visuals = match self.mode {
            ThemeMode::Dark => Visuals::dark(),
            ThemeMode::Light => Visuals::light(),
        };

        visuals.panel_fill = self.surface_1;
        // Floating things (menus, popups, windows) sit a clear step above the
        // panels, rerun-style, with a hairline edge and a genuine drop shadow.
        visuals.window_fill = self.surface_3;
        visuals.window_stroke = Stroke::new(1.0_f32, self.hairline);
        visuals.extreme_bg_color = self.surface_0;
        visuals.faint_bg_color = self.surface_2;
        visuals.code_bg_color = self.surface_0;

        visuals.widgets.noninteractive.bg_fill = self.surface_1;
        visuals.widgets.noninteractive.weak_bg_fill = self.surface_1;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, self.hairline);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, self.text_secondary);

        // Idle widgets carry no border; hover and press bring one back (owner
        // amendment to K-084) so state reads from fill *and* edge together.
        visuals.widgets.inactive.bg_fill = self.surface_3;
        visuals.widgets.inactive.weak_bg_fill = self.surface_2;
        visuals.widgets.inactive.bg_stroke = Stroke::NONE;
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, self.text_secondary);

        visuals.widgets.hovered.bg_fill = self.surface_4;
        visuals.widgets.hovered.weak_bg_fill = self.surface_4;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, self.hairline_strong);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, self.text_primary);

        visuals.widgets.active.bg_fill = self.hairline_strong;
        visuals.widgets.active.weak_bg_fill = self.hairline_strong;
        visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, self.accent);
        visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, self.text_primary);

        visuals.widgets.open.bg_fill = self.surface_3;
        visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, self.hairline_strong);
        visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, self.text_primary);

        // Selection is punchy, rerun-style — the accent carries it.
        visuals.selection.bg_fill = self.accent.gamma_multiply(0.5);
        visuals.selection.stroke = Stroke::new(1.0_f32, self.accent);
        visuals.hyperlink_color = self.accent;
        visuals.warn_fg_color = self.warning;
        visuals.error_fg_color = self.error;

        // Radii: control/float pair from the shape tokens (K-092) — 4px/6px
        // under Sharp (rerun's small/window pair, unchanged), larger under
        // Round so a control doesn't look unfinished inside a rounded card.
        let r = CornerRadius::same(self.tokens.control_radius);
        visuals.widgets.noninteractive.corner_radius = r;
        visuals.widgets.inactive.corner_radius = r;
        visuals.widgets.hovered.corner_radius = r;
        visuals.widgets.active.corner_radius = r;
        visuals.widgets.open.corner_radius = r;
        visuals.window_corner_radius = CornerRadius::same(self.tokens.float_radius);
        visuals.menu_corner_radius = CornerRadius::same(self.tokens.float_radius);

        // Floats cast a real shadow (rerun: offset 0/15, blur 50) — panels
        // still separate by hairline only, so depth reads only where something
        // genuinely floats. Shape-independent: this is the FLOAT shadow
        // (menus/dialogs); a docked pane's own card shadow under Round is a
        // separate, smaller `self.tokens.card_shadow`, applied directly by
        // `DockBehavior::pane_ui` in shell.rs, not through this style.
        visuals.popup_shadow = egui::Shadow {
            offset: [0, 15],
            blur: 50,
            spread: 0,
            color: Color32::from_black_alpha(0x80),
        };
        visuals.window_shadow = visuals.popup_shadow;

        let mut style = (*ctx.style()).clone();
        style.visuals = visuals;

        // Pro-density type scale (docs/15-DESIGN.md §density): 12 px body, 11 px small.
        use egui::{FontFamily, FontId, TextStyle};
        style.text_styles = [
            (
                TextStyle::Heading,
                FontId::new(16.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(12.0, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(12.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Small,
                FontId::new(11.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(12.0, FontFamily::Monospace),
            ),
        ]
        .into();
        // Denser than rerun's 8×8 grid on purpose: the timeline's row pitch is
        // part of Lumit's feel and stays put. The rest of their metrics land
        // as-is: 14 px indent, 16 px interact height, roomy 12 px menu margins,
        // thin solid 6 px scrollbars.
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(8.0, 3.0);
        style.spacing.indent = 14.0;
        style.spacing.interact_size.y = 16.0;
        style.spacing.menu_margin = egui::Margin::same(12);
        style.spacing.scroll = egui::style::ScrollStyle::solid();
        style.spacing.scroll.bar_width = 6.0;
        style.spacing.scroll.bar_inner_margin = 2.0;
        style.spacing.scroll.bar_outer_margin = 2.0;

        // egui turns its own developer overlays on in debug builds — the
        // orange "unaligned" lines on sub-pixel widget edges are the visible
        // one. Lumit's debug build is what the owner runs day to day, so the
        // dev overlay is just noise here; switch it off.
        style.debug.show_unaligned = false;

        ctx.set_style(style);
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

impl Theme {
    /// This theme with a user-picked accent (the customisation seed): the
    /// accent and its hover take the picked colour; selection, playhead and
    /// active states follow automatically since they all read `accent`.
    /// Lives here because only this module constructs colours.
    ///
    /// The hover shift direction depends on `self.mode` (K-092): brightening
    /// reads as "more prominent" on a dark surface, so Dark brightens; a
    /// light surface needs the opposite to read as more prominent, so Light
    /// darkens by the same amount instead.
    pub fn with_accent(mut self, rgb: [u8; 3]) -> Self {
        self.accent = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
        let shift = |c: u8| match self.mode {
            ThemeMode::Dark => c.saturating_add(0x12),
            ThemeMode::Light => c.saturating_sub(0x12),
        };
        self.accent_hover = Color32::from_rgb(shift(rgb[0]), shift(rgb[1]), shift(rgb[2]));
        self
    }

    /// The default accent as plain RGB, for seeding the picker.
    pub const fn default_accent_rgb() -> [u8; 3] {
        [0xe0, 0x5a, 0x72]
    }
}

/// Which background ramp the user has picked (the seed of the full theme
/// picker to come). `Dark` is the K-084 rerun-structure ramp; `DarkBlue` is
/// the previous, bluer and slightly lighter ramp, kept as an option by
/// owner request.
#[derive(Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ThemeVariant {
    #[default]
    Dark,
    DarkBlue,
}

impl Theme {
    /// The theme for a variant choice.
    pub const fn of(variant: ThemeVariant) -> Self {
        match variant {
            ThemeVariant::Dark => Self::dark(),
            ThemeVariant::DarkBlue => Self::dark_blue(),
        }
    }

    /// The pre-K-084 ramp: bluer, a step lighter — everything else (accent,
    /// roles, curves, layer colours, and the whole widget/spacing system in
    /// `apply`) is shared with `dark`. The Viewer surround stays strictly
    /// neutral here too.
    pub const fn dark_blue() -> Self {
        Self {
            surface_0: Color32::from_rgb(0x14, 0x16, 0x18),
            surface_1: Color32::from_rgb(0x1b, 0x1e, 0x20),
            surface_2: Color32::from_rgb(0x22, 0x26, 0x2a),
            surface_3: Color32::from_rgb(0x2b, 0x30, 0x34),
            surface_4: Color32::from_rgb(0x34, 0x3a, 0x3f),
            viewer_surround: Color32::from_rgb(0x1e, 0x1e, 0x1e),
            text_primary: Color32::from_rgb(0xe6, 0xe9, 0xea),
            text_secondary: Color32::from_rgb(0xb6, 0xbc, 0xbf),
            text_muted: Color32::from_rgb(0x83, 0x8b, 0x90),
            text_disabled: Color32::from_rgb(0x66, 0x70, 0x77),
            hairline: Color32::from_rgba_premultiplied(0x25, 0x27, 0x29, 0xff),
            hairline_strong: Color32::from_rgba_premultiplied(0x3d, 0x40, 0x42, 0xff),
            ..Self::dark()
        }
    }
}

/// A colour that comes from the *document* (a solid's swatch, a comp
/// background) rather than the design system. Lives here because this module
/// is the only place allowed to construct egui colours (design lint).
pub fn document_colour(rgba: [u8; 4]) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3])
}

/// How much UI-chrome motion to show (K-092, owner request): collapsing
/// headers, resizable-panel expand/collapse, scrollbar fade, dialog
/// fade-in — the handful of things egui's own internals animate. Does NOT
/// touch the user's own timeline/keyframe animation, and does not
/// retroactively animate Lumit's own menus/dropdowns (`ui.menu_button`
/// throughout the app), which have no animation today regardless of this
/// setting — egui's popup system has no animate hook (docs/15-DESIGN.md §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum AnimationLevel {
    #[default]
    All,
    Minimal,
    None,
}

/// Apply the animation-level setting: a global lever over `Style::
/// animation_time`, the one knob everything egui itself animates reads as
/// its default duration. `None` also clears any animation already in
/// flight, so the transition itself is instant rather than coasting to
/// zero at the old speed.
pub fn apply_animation_level(ctx: &egui::Context, level: AnimationLevel) {
    let time = match level {
        // Egui's own stock default is ~0.083s; a touch above it stays
        // within the ≤150ms micro-motion budget (15-DESIGN §8).
        AnimationLevel::All => 0.12,
        AnimationLevel::Minimal => 0.05,
        AnimationLevel::None => 0.0,
    };
    ctx.style_mut(|s| s.animation_time = time);
    if level == AnimationLevel::None {
        ctx.clear_animations();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// `ShapeTokens::SHARP` must reproduce the exact numbers `apply()`
    /// hardcoded before K-092, so picking Sharp is provably a no-op.
    #[test]
    fn shape_tokens_sharp_matches_the_pre_k092_hardcoded_numbers() {
        let t = ShapeTokens::SHARP;
        assert_eq!(t.control_radius, 4);
        assert_eq!(t.float_radius, 6);
        assert_eq!(t.card_radius, 0);
        assert_eq!(t.card_padding, 0);
        assert_eq!(t.tile_gap, 1.0);
        assert_eq!(t.window_inset, 0.0);
        assert_eq!(t.card_shadow, Shadow::NONE);
    }

    /// `for_settings` composes the three axes correctly: Dark+Sharp matches
    /// plain `dark()`; Light ignores which `ThemeVariant` is picked (there
    /// is one light ramp); Round swaps in `ShapeTokens::ROUND` regardless
    /// of mode.
    #[test]
    fn for_settings_composes_mode_variant_and_shape() {
        let base = Theme::dark();
        let composed = Theme::for_settings(ThemeMode::Dark, ThemeVariant::Dark, ThemeShape::Sharp);
        assert_eq!(composed.surface_1, base.surface_1);
        assert_eq!(composed.accent, base.accent);
        assert_eq!(composed.tokens, ShapeTokens::SHARP);

        let light_dark =
            Theme::for_settings(ThemeMode::Light, ThemeVariant::Dark, ThemeShape::Sharp);
        let light_darkblue =
            Theme::for_settings(ThemeMode::Light, ThemeVariant::DarkBlue, ThemeShape::Sharp);
        assert_eq!(
            light_dark.surface_1, light_darkblue.surface_1,
            "the dark-ramp variant pick must not affect Light mode"
        );
        assert_eq!(light_dark.mode, ThemeMode::Light);

        let round = Theme::for_settings(ThemeMode::Dark, ThemeVariant::Dark, ThemeShape::Round);
        assert_eq!(round.tokens, ShapeTokens::ROUND);
        assert_eq!(round.shape, ThemeShape::Round);
        // Colours are unaffected by shape.
        assert_eq!(round.surface_1, base.surface_1);
    }

    /// `with_accent`'s hover shift must invert by mode: brightening on Dark
    /// (existing behaviour), darkening on Light (K-092) — same magnitude,
    /// opposite direction, so a light-mode hover doesn't wash out to white.
    #[test]
    fn with_accent_hover_shift_direction_differs_by_mode() {
        let rgb = [0x80, 0x40, 0x60];
        let dark = Theme::dark().with_accent(rgb);
        assert_eq!(dark.accent_hover, Color32::from_rgb(0x92, 0x52, 0x72));

        let light = Theme::light().with_accent(rgb);
        assert_eq!(light.accent_hover, Color32::from_rgb(0x6e, 0x2e, 0x4e));
    }

    /// The Round card's padding must clear the geometric minimum that keeps
    /// a content rect's square corner from poking past the card's rounded
    /// silhouette (inset ≥ radius × (1 − 1/√2)) — pins the relationship so a
    /// future radius tweak can't silently reintroduce corner bleed.
    #[test]
    fn round_card_padding_clears_corner_bleed() {
        let t = ShapeTokens::ROUND;
        let min_padding = f32::from(t.card_radius) * (1.0 - 1.0 / std::f32::consts::SQRT_2);
        assert!(
            f32::from(t.card_padding) >= min_padding,
            "card_padding {} must be >= {min_padding} for radius {}",
            t.card_padding,
            t.card_radius
        );
    }
}
