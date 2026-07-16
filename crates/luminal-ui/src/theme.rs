//! The Luminal theme: a rerun-inspired dark system (K-084), per
//! docs/15-DESIGN.md — Luminal's own hues on rerun.io's structure: a
//! near-black canvas, panels barely above it, floating surfaces a clear step
//! up, borderless widgets whose states are fill changes, crisp 1 px hairlines,
//! and thin solid scrollbars.
//!
//! This module is the ONLY place in the codebase where colour hex values may
//! appear (enforced by CI grep). Everything else reads the `Theme` struct.
//! Font embedding (Inter per the design doc) is a follow-up pending the owner's
//! nod on shipping the font file; until then egui's defaults stand in.

use egui::{Color32, CornerRadius, Stroke, Visuals};

/// Semantic colour tokens. Names mirror docs/15-DESIGN.md §tokens.
#[derive(Clone, Copy)]
pub struct Theme {
    // Surfaces (near-neutral dark ramp)
    pub surface_0: Color32,
    pub surface_1: Color32,
    pub surface_2: Color32,
    pub surface_3: Color32,
    pub surface_4: Color32,
    /// The Viewer pasteboard — exactly neutral, R = G = B (grading accuracy).
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
    /// Graph-editor curve strokes (15-DESIGN §graph: the viz ramp).
    pub curve: [Color32; 4],
    /// Layer-type identity colours (15-DESIGN §6.1).
    pub layer: LayerColours,
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
            // The rerun-structure ramp (K-084) in Luminal's cool grey: the
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

    /// Apply the theme to an egui context: visuals, spacing, type scale — the
    /// rerun-inspired system (K-084). Widgets are borderless: their states are
    /// fill changes, not stroke changes; menus float on a soft real shadow;
    /// scrollbars are thin and solid.
    pub fn apply(&self, ctx: &egui::Context) {
        let mut visuals = Visuals::dark();

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

        // Interactive widgets carry no border; state reads from the fill alone.
        visuals.widgets.inactive.bg_fill = self.surface_3;
        visuals.widgets.inactive.weak_bg_fill = self.surface_2;
        visuals.widgets.inactive.bg_stroke = Stroke::NONE;
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, self.text_secondary);

        visuals.widgets.hovered.bg_fill = self.surface_4;
        visuals.widgets.hovered.weak_bg_fill = self.surface_4;
        visuals.widgets.hovered.bg_stroke = Stroke::NONE;
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, self.text_primary);

        visuals.widgets.active.bg_fill = self.hairline_strong;
        visuals.widgets.active.weak_bg_fill = self.hairline_strong;
        visuals.widgets.active.bg_stroke = Stroke::NONE;
        visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, self.text_primary);

        visuals.widgets.open.bg_fill = self.surface_3;
        visuals.widgets.open.bg_stroke = Stroke::NONE;
        visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, self.text_primary);

        // Selection is punchy, rerun-style — the accent carries it.
        visuals.selection.bg_fill = self.accent.gamma_multiply(0.5);
        visuals.selection.stroke = Stroke::new(1.0_f32, self.accent);
        visuals.hyperlink_color = self.accent;
        visuals.warn_fg_color = self.warning;
        visuals.error_fg_color = self.error;

        // Radii: 4 px controls, 6 px floats (rerun's small/window pair).
        let r = CornerRadius::same(4);
        visuals.widgets.noninteractive.corner_radius = r;
        visuals.widgets.inactive.corner_radius = r;
        visuals.widgets.hovered.corner_radius = r;
        visuals.widgets.active.corner_radius = r;
        visuals.widgets.open.corner_radius = r;
        visuals.window_corner_radius = CornerRadius::same(6);
        visuals.menu_corner_radius = CornerRadius::same(6);

        // Floats cast a real shadow (rerun: offset 0/15, blur 50) — panels
        // still separate by hairline only, so depth reads only where something
        // genuinely floats.
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
        // part of Luminal's feel and stays put. The rest of their metrics land
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

        ctx.set_style(style);
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

/// A colour that comes from the *document* (a solid's swatch, a comp
/// background) rather than the design system. Lives here because this module
/// is the only place allowed to construct egui colours (design lint).
pub fn document_colour(rgba: [u8; 4]) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3])
}
