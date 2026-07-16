//! The Luminal theme: dark-first Aizome, per docs/15-DESIGN.md.
//!
//! This module is the ONLY place in the codebase where colour hex values may
//! appear (enforced by CI grep). Everything else reads the `Theme` struct.
//! Font embedding (Schibsted Grotesk / Inter / JetBrains Mono per the design
//! doc) is a Phase 0 follow-up; until then egui's defaults stand in.

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

            // text_primary at 8% / 18% over the ramp
            hairline: Color32::from_rgba_premultiplied(0x25, 0x27, 0x29, 0xff),
            hairline_strong: Color32::from_rgba_premultiplied(0x3d, 0x40, 0x42, 0xff),

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

    /// Apply the theme to an egui context: visuals, spacing, type scale.
    pub fn apply(&self, ctx: &egui::Context) {
        let mut visuals = Visuals::dark();

        visuals.panel_fill = self.surface_1;
        visuals.window_fill = self.surface_1;
        visuals.extreme_bg_color = self.surface_0;
        visuals.faint_bg_color = self.surface_2;
        visuals.code_bg_color = self.surface_0;

        visuals.widgets.noninteractive.bg_fill = self.surface_1;
        visuals.widgets.noninteractive.weak_bg_fill = self.surface_1;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, self.hairline);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, self.text_secondary);

        visuals.widgets.inactive.bg_fill = self.surface_2;
        visuals.widgets.inactive.weak_bg_fill = self.surface_2;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, self.hairline);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, self.text_secondary);

        visuals.widgets.hovered.bg_fill = self.surface_3;
        visuals.widgets.hovered.weak_bg_fill = self.surface_3;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, self.hairline_strong);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, self.text_primary);

        visuals.widgets.active.bg_fill = self.surface_4;
        visuals.widgets.active.weak_bg_fill = self.surface_4;
        visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, self.accent);
        visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, self.text_primary);

        visuals.widgets.open.bg_fill = self.surface_3;
        visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, self.hairline_strong);
        visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, self.text_primary);

        visuals.selection.bg_fill = self.accent.gamma_multiply(0.35);
        visuals.selection.stroke = Stroke::new(1.0_f32, self.accent);
        visuals.hyperlink_color = self.accent;
        visuals.warn_fg_color = self.warning;
        visuals.error_fg_color = self.error;

        // Radii: 4 px controls (household 4/8/16 scale; panels get 8 via dock style).
        let r = CornerRadius::same(4);
        visuals.widgets.noninteractive.corner_radius = r;
        visuals.widgets.inactive.corner_radius = r;
        visuals.widgets.hovered.corner_radius = r;
        visuals.widgets.active.corner_radius = r;
        visuals.widgets.open.corner_radius = r;
        visuals.window_corner_radius = CornerRadius::same(8);

        // Hairline elevation: no popup shadows beyond a whisper; true floats keep theirs.
        visuals.popup_shadow.blur = 8;
        visuals.popup_shadow.offset = [0, 2];
        visuals.window_shadow.blur = 16;
        visuals.window_shadow.offset = [0, 4];

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
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(10.0, 4.0);

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
