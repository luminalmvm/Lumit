//! `shell::inspector` — property-row rendering for the Timeline layer
//! area and the Effect Controls panel. Split out of a single large file
//! (mechanical, no behaviour change): this module keeps the shared row
//! context (`RowCtx`), the row scaffolding (`row_frame`, the section bar)
//! and a handful of small helpers, and re-exports the topic submodules so
//! every existing `shell::…` path still resolves. Shared shell names reach
//! the submodules through `use super::*` and these glob re-exports.

use super::*;

mod channel_picker;
mod controls;
mod effect_rows;
mod keyframe_nav;
mod lane;
mod speed_rows;
mod transform_rows;

pub(crate) use channel_picker::*;
pub(crate) use controls::*;
pub(crate) use effect_rows::*;
pub(crate) use keyframe_nav::*;
pub(crate) use lane::*;
pub(crate) use speed_rows::*;
pub(crate) use transform_rows::*;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

/// Read-only context shared by every property row in a layer's twirl-down.
pub(crate) struct RowCtx<'a> {
    pub(crate) theme: &'a Theme,
    pub(crate) comp_id: uuid::Uuid,
    /// The composition, for rows that need its other layers (e.g. a Layer
    /// effect parameter's picker — K-123).
    pub(crate) comp: &'a lumit_core::model::Composition,
    pub(crate) layer: &'a lumit_core::model::Layer,
    pub(crate) lt: f64,
    pub(crate) off: f64,
    pub(crate) fps: f64,
    /// The lane scroll viewport, so property-row outlines clip to their own x
    /// but the viewport's y (no vertical bleed when a row is half-scrolled).
    pub(crate) viewport: egui::Rect,
    pub(crate) track_left: f32,
    pub(crate) track_w: f32,
    /// The displayed time axis (zoom + scroll), so property-row keyframe
    /// diamonds sit exactly under the layer bars at any zoom.
    pub(crate) px_per_sec: f64,
    pub(crate) view_start: f64,
    /// True in graph mode (K-070): the outline half of every row still draws,
    /// but nothing is painted on the lane side — the curve owns that area.
    pub(crate) graph_mode: bool,
    /// Whether `effects_rows` draws its Add effect / Presets toolbar row
    /// (owner): true in the Effect Controls panel, false in the timeline —
    /// effects are added there by drag, right-click or the palette instead.
    pub(crate) effects_toolbar: bool,
    /// The anchor of the property-row selection (note 2.8.1) — the most recently
    /// clicked row, which a Shift-click ranges to and the graph follows.
    pub(crate) selected_prop: Option<crate::app_state::PropSel>,
    /// The full highlighted set (note 2.6b): plain click picks one, Ctrl-click
    /// toggles, Shift-click ranges. Every row in it lifts its background. Owned
    /// (a cheap per-frame clone) so the row functions can still take `&mut app`.
    pub(crate) selected_props: Vec<crate::app_state::PropSel>,
}

impl RowCtx<'_> {
    /// Whether `row` on this row's layer is highlighted (notes 2.8.1/2.6): it is
    /// in the selection set, or it is the anchor (any code path that sets only
    /// the anchor still highlights).
    pub(crate) fn is_selected(&self, row: crate::app_state::PropRow) -> bool {
        let ps = crate::app_state::PropSel {
            layer: self.layer.id,
            row,
        };
        self.selected_props.contains(&ps) || self.selected_prop == Some(ps)
    }
}

/// Snap the row cursor to just right of the control column's midline (owner):
/// property values sit on one shared vertical line down the middle of the
/// column, however long their labels run. A no-op when the label has already
/// passed the middle — the value then packs on as before rather than clipping.
pub(crate) fn snap_to_value_column(c: &mut egui::Ui) {
    let mid = (c.max_rect().left() + c.max_rect().right()) * 0.5;
    let cur = c.cursor().left();
    if mid > cur {
        c.add_space(mid - cur);
    }
}

/// True when the pointer clicked anywhere within `row_rect` this frame — the
/// whole-row hit test behind row selection (note 2.8.1). A plain click on the
/// row's controls still works them; this fires alongside so the row highlights
/// wherever you click it. A drag (value scrub, key drag) is not a click, so it
/// never trips selection.
pub(crate) fn row_click(ui: &egui::Ui, row_rect: egui::Rect) -> bool {
    ui.rect_contains_pointer(row_rect) && ui.input(|i| i.pointer.primary_clicked())
}

/// The fill an effect-title bar paints. `surface_2` deliberately, not
/// `surface_1`: the Effect Controls panel's card is filled with `surface_1`
/// under the Round shape and the Sharp background (`surface_0`) sits within a
/// few RGB steps of it — a `surface_1` bar was therefore invisible in the one
/// place it mattered most, which is exactly the defect the owner reported.
/// `surface_2` is the same visible step the ruler and the selection highlight
/// use, so the bar reads on every scheme and both shapes.
pub(crate) fn section_bar_fill(theme: &Theme) -> egui::Color32 {
    theme.surface_2
}

/// Paint a full-width themed bar behind an effect's title row — the one shared
/// helper for both of its homes (Mack): the Effect Controls panel and the
/// layer area's Effects group both draw their title rows through
/// `effects_rows`, so this bar is what makes each effect's start obvious in
/// both. Drawn under the row's widgets, clipped via the scroll viewport like
/// every other left-column paint. In the Timeline the bar spans the outline
/// column (up to the lane divider); in the panel (no lane, `track_w == 0`)
/// it spans the whole panel width.
pub(crate) fn section_bar(ui: &egui::Ui, ctx: &RowCtx, row_rect: egui::Rect, highlight: bool) {
    let mut p = ui.painter().clone();
    p.set_clip_rect(ctx.viewport);
    let edge = if ctx.track_w > 0.0 {
        ctx.track_left - 6.0
    } else {
        ctx.track_left
    };
    let right = edge.max(row_rect.left() + 1.0);
    // A brighter fill when one of this effect's params is the highlighted row
    // (note 2.8.2) — the plain bar is already surface_2, so the highlight lifts
    // one step further to read against it.
    let fill = if highlight {
        ctx.theme.surface_3
    } else {
        section_bar_fill(ctx.theme)
    };
    p.rect_filled(
        egui::Rect::from_min_max(row_rect.min, egui::pos2(right, row_rect.bottom())),
        2.0,
        fill,
    );
}

/// The height of one property row on the timeline. Grew 20 → 22 (T1, pair
/// value-box headroom) → 24 (owner): the row-bottom divider hairline needs a
/// few clear pixels below the ~18 px value boxes and the pair rows' link glyph,
/// or they sit on the line. Content is vertically centred, so the extra height
/// becomes breathing room above and below.
pub(crate) const ROW_H: f32 = 24.0;

/// Allocate one property timeline row (`ROW_H` tall) and return (row_rect,
/// left-column child ui). The child is clipped so widgets never spill into the
/// track area.
pub(crate) fn row_frame(
    ui: &mut egui::Ui,
    ctx: &RowCtx,
    highlight: bool,
) -> (egui::Rect, egui::Ui) {
    let (row_rect, _resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_H),
        egui::Sense::hover(),
    );
    if highlight {
        // Left of the lanes → replace the clip; with_clip_rect would intersect the
        // lane clip and hide this highlight.
        let mut hp = ui.painter().clone();
        hp.set_clip_rect(ctx.viewport);
        hp.rect_filled(
            egui::Rect::from_min_max(
                row_rect.min,
                egui::pos2(ctx.track_left - 6.0, row_rect.bottom()),
            ),
            2.0,
            ctx.theme.surface_2,
        );
    }
    // A hairline under each property row (EC1 / TL1; owner: it also crosses the
    // lane area, so a keyframe reads against its property at a glance). In graph
    // mode the curve owns the lane side, so the line stays on the outline there
    // (which, in the Effect Controls panel — no lane — is the whole panel).
    {
        let mut dp = ui.painter().clone();
        dp.set_clip_rect(ctx.viewport);
        let right = if ctx.graph_mode {
            (ctx.track_left - 6.0).max(row_rect.left())
        } else {
            row_rect.right()
        };
        dp.hline(
            row_rect.left()..=right,
            row_rect.bottom() - 0.5_f32,
            egui::Stroke::new(1.0_f32, ctx.theme.hairline),
        );
    }
    let left_rect = egui::Rect::from_min_max(
        egui::pos2(row_rect.left() + 24.0, row_rect.top()),
        egui::pos2(
            (ctx.track_left - 6.0).max(row_rect.left() + 25.0),
            row_rect.bottom(),
        ),
    );
    let mut c = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(left_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    // Clip to the outline column, but bounded by the scroll viewport's y so a
    // half-scrolled property row doesn't bleed past the ruler.
    c.set_clip_rect(left_rect.intersect(ctx.viewport));
    (row_rect, c)
}

pub(crate) fn mask_space(
    layer: &lumit_core::model::Layer,
    app: &AppState,
    comp: &lumit_core::model::Composition,
) -> (f64, f64) {
    match &layer.kind {
        // An adjustment layer is comp-sized: its masks live in comp space.
        lumit_core::model::LayerKind::Adjustment => (f64::from(comp.width), f64::from(comp.height)),
        lumit_core::model::LayerKind::Solid { def } => app
            .store
            .snapshot()
            .solid(*def)
            .map(|sd| (f64::from(sd.width), f64::from(sd.height)))
            .unwrap_or((f64::from(comp.width), f64::from(comp.height))),
        lumit_core::model::LayerKind::Precomp { comp: nested } => app
            .store
            .snapshot()
            .comp(*nested)
            .map(|n| (f64::from(n.width), f64::from(n.height)))
            .unwrap_or((f64::from(comp.width), f64::from(comp.height))),
        lumit_core::model::LayerKind::Camera { .. }
        | lumit_core::model::LayerKind::Sequence { .. }
        | lumit_core::model::LayerKind::Text { .. } => {
            (f64::from(comp.width), f64::from(comp.height))
        }
        #[cfg(feature = "media")]
        lumit_core::model::LayerKind::Footage { item, .. } => match app.media.map.get(item) {
            Some(crate::app_state::media::MediaStatus::Ready { probe, .. }) => probe
                .video
                .as_ref()
                .map(|v| (f64::from(v.width), f64::from(v.height)))
                .unwrap_or((f64::from(comp.width), f64::from(comp.height))),
            _ => (f64::from(comp.width), f64::from(comp.height)),
        },
        #[cfg(not(feature = "media"))]
        lumit_core::model::LayerKind::Footage { .. } => {
            (f64::from(comp.width), f64::from(comp.height))
        }
    }
}

/// The single `model::BlendMode` → `gpu::Blend` mapping shared by the preview
/// and the export path (K-031: they must never disagree). Every mode maps to
/// its like-named GPU variant (K-162, T24).
#[cfg(feature = "media")]
pub(crate) fn blend_of(b: lumit_core::model::BlendMode) -> lumit_gpu::Blend {
    use lumit_core::model::BlendMode as M;
    use lumit_gpu::Blend as G;
    match b {
        M::Normal => G::Normal,
        M::Add => G::Add,
        M::Multiply => G::Multiply,
        M::Screen => G::Screen,
        M::Overlay => G::Overlay,
        M::SoftLight => G::SoftLight,
        M::HardLight => G::HardLight,
        M::Lighten => G::Lighten,
        M::Darken => G::Darken,
        M::Subtract => G::Subtract,
        M::ColourBurn => G::ColourBurn,
        M::LinearBurn => G::LinearBurn,
        M::DarkerColour => G::DarkerColour,
        M::ColourDodge => G::ColourDodge,
        M::LighterColour => G::LighterColour,
        M::LinearLight => G::LinearLight,
        M::VividLight => G::VividLight,
        M::PinLight => G::PinLight,
        M::HardMix => G::HardMix,
        M::Difference => G::Difference,
        M::Exclusion => G::Exclusion,
        M::Divide => G::Divide,
        M::Hue => G::Hue,
        M::Saturation => G::Saturation,
        M::Colour => G::Colour,
        M::Luminosity => G::Luminosity,
    }
}

/// Layer time → rational on the flick grid (the only f64→rational route).
/// Clamps to ≥ 0: layer-local times (keyframes, trim edges) never precede 0.
pub(crate) fn rational_at(seconds: f64) -> lumit_core::Rational {
    lumit_core::Rational::from_f64_on_grid(seconds.max(0.0), lumit_core::Rational::FLICK_DEN)
        .unwrap_or(lumit_core::Rational::ZERO)
}

/// Comp time → rational on the flick grid, sign preserved (K-153). A layer's
/// placement (in/out point and start offset) may sit before comp time 0, so the
/// move drag and its `moved_span` convert through this instead of `rational_at`,
/// which clamps to 0. Only the portion overlapping [0, comp_end) is rendered.
pub(crate) fn rational_at_signed(seconds: f64) -> lumit_core::Rational {
    lumit_core::Rational::from_f64_on_grid(seconds, lumit_core::Rational::FLICK_DEN)
        .unwrap_or(lumit_core::Rational::ZERO)
}

/// Insert or replace a keyframe at layer time `lt` with `value`, keeping the
/// list sorted and times unique (half-frame tolerance for "same time").
pub(crate) fn upsert_key(
    slot: &lumit_core::anim::Property,
    lt: f64,
    value: f64,
) -> Vec<lumit_core::anim::Keyframe> {
    use lumit_core::anim::{Animation, Keyframe, SideInterp};
    let mut keys = match &slot.animation {
        Animation::Keyframed(k) => k.clone(),
        Animation::Static(v) => vec![Keyframe {
            time: rational_at(0.0),
            value: *v,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        }],
    };
    const EPS: f64 = 1.0 / 240.0;
    if let Some(existing) = keys.iter_mut().find(|k| (k.time.to_f64() - lt).abs() < EPS) {
        existing.value = value;
    } else {
        keys.push(Keyframe {
            time: rational_at(lt),
            value,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        });
        keys.sort_by_key(|k| k.time);
    }
    keys
}
