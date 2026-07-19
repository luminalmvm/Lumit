//! `shell::eyedropper` — the colour/depth eyedropper: a small tool that arms
//! from an effect parameter's button and then samples a value from the pixels
//! shown in the Viewer.
//!
//! In plain terms: click the little dropper next to a colour (or the
//! depth-of-field Focus), and the Viewer grows a magnifier that follows the
//! cursor. It shows a zoomed grid of the pixels under the pointer; click to
//! pick. The picked value is written back through the normal undoable edit
//! path, exactly as if you had typed it. Shift+scroll while picking widens the
//! sampled area so a noisy patch averages out instead of grabbing one grainy
//! pixel.
//!
//! The pixels come from the same place the Scopes panel reads: the composited
//! frame the Viewer last banked in the RAM cache (display-ready sRGB bytes).
//! Colours are converted back to the parameter's scene-linear space on commit.

use super::*;
use crate::app_state::EyedropperTarget;

/// The largest averaging region (side length in pixels); also the magnifier's
/// grid, so the region never overflows what is shown.
#[cfg(feature = "media")]
const MAX_REGION: u32 = 9;

/// The context key a pending arm request travels under. The button lives deep
/// inside `effects_rows` (shared with the Timeline, whose signature must not
/// change), so it stashes its request here and the shell drains it once per
/// frame with [`take_arm_request`].
fn arm_request_id() -> egui::Id {
    egui::Id::new("lumit::eyedropper::arm-request")
}

/// Record that an eyedropper button was clicked: arms the given target next
/// frame, once the shell drains it.
pub(crate) fn request_arm(ctx: &egui::Context, target: EyedropperTarget) {
    ctx.data_mut(|d| d.insert_temp(arm_request_id(), target));
}

/// Take any pending arm request (drained once per frame by the shell).
pub(crate) fn take_arm_request(ctx: &egui::Context) -> Option<EyedropperTarget> {
    ctx.data_mut(|d| {
        let req = d.get_temp::<EyedropperTarget>(arm_request_id());
        if req.is_some() {
            d.remove::<EyedropperTarget>(arm_request_id());
        }
        req
    })
}

/// The decoded pixels of the depth layer a DoF Focus pick reads: `(width,
/// height, rgba)`. Stashed here by the shell (which alone holds the per-layer
/// decode cache) so the app-only [`viewer_overlay`] can sample true depth
/// instead of the composite. An `Arc` so the per-frame stash is a pointer bump,
/// not a pixel copy.
#[cfg(feature = "media")]
type DepthSource = std::sync::Arc<(u32, u32, Vec<u8>)>;

fn depth_pixels_id() -> egui::Id {
    egui::Id::new("lumit::eyedropper::depth-pixels")
}

fn depth_stamp_id() -> egui::Id {
    egui::Id::new("lumit::eyedropper::depth-stamp")
}

/// While a DoF Focus (depth) pick is armed, stash the referenced depth layer's
/// own decoded pixels so the pick samples the real depth pass, not the composite
/// (which never shows a hidden depth layer). Called by the shell, the only place
/// that holds both the egui context and the per-layer decode cache
/// (`last_comp`). The clone happens once per (frame, depth-layer) — a stamp
/// guards it — so hovering the magnifier is a pointer read, not a re-copy. A
/// no-op unless a depth pick is armed and its depth layer has decoded.
#[cfg(feature = "media")]
pub(crate) fn stash_depth_source(
    ctx: &egui::Context,
    app: &AppState,
    last_comp: &Option<crate::app_state::preview::CompFrame>,
) {
    let Some(target) = app.eyedropper else {
        return;
    };
    if !matches!(target.mode, crate::app_state::EyedropperMode::Depth) {
        return;
    }
    let Some(cf) = last_comp else {
        return;
    };
    // Resolve the depth layer referenced by the DoF effect this pick edits.
    let doc = app.store.snapshot();
    let depth_id = app
        .preview_comp
        .and_then(|c| doc.comp(c))
        .and_then(|comp| comp.layers.iter().find(|l| l.id == target.layer))
        .filter(|l| target.effect < l.effects.len())
        .and_then(|l| l.effects[target.effect].layer_ref("depth"));
    let Some(depth_id) = depth_id else {
        return;
    };
    let Some(lp) = cf.layers.iter().find(|l| l.layer == depth_id) else {
        return;
    };
    // Only re-clone when the frame or the depth layer changes.
    let stamp = (cf.frame, depth_id);
    let cur = ctx.data(|d| d.get_temp::<(usize, uuid::Uuid)>(depth_stamp_id()));
    if cur != Some(stamp) {
        let pixels: DepthSource = std::sync::Arc::new((lp.width, lp.height, lp.rgba.clone()));
        ctx.data_mut(|d| {
            d.insert_temp(depth_stamp_id(), stamp);
            d.insert_temp(depth_pixels_id(), pixels);
        });
    }
}

/// Paint the eyedropper glyph in `rect`, in `color` — Iconoir's `color-picker`
/// (an eyedropper/pipette), via the shared icon painter. The colour comes from
/// the theme.
pub(crate) fn paint_icon(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
    crate::icons::paint(painter, rect, crate::icons::Icon::Eyedropper, color, 1.4);
}

/// The armed eyedropper's Viewer overlay: the magnifier, the region control,
/// and the sample-on-click commit. A no-op unless armed. `draw` is the image's
/// on-screen rect, `image_area` the Viewer's image region (for cancel/clamp),
/// and `view` the Viewer's own interaction (its click samples).
#[cfg(feature = "media")]
pub(crate) fn viewer_overlay(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut AppState,
    draw: egui::Rect,
    image_area: egui::Rect,
    view: &egui::Response,
) {
    let Some(target) = app.eyedropper else {
        return;
    };

    // Escape cancels outright.
    if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
        app.eyedropper = None;
        return;
    }

    let Some(cursor) = ui.ctx().input(|i| i.pointer.hover_pos()) else {
        app.eyedropper_primed = true;
        return;
    };
    let over_image = draw.contains(cursor);
    let over_viewer = image_area.contains(cursor);

    // Shift+scroll grows or shrinks the averaged region. With Shift held, most
    // platforms deliver the wheel on the X axis (it becomes a horizontal scroll),
    // so take whichever axis actually carries the motion — reading only `.y`
    // meant the size never changed while Shift was down (owner-reported bug).
    let (shift, dx, dy) = ui.ctx().input(|i| {
        (
            i.modifiers.shift,
            i.raw_scroll_delta.x,
            i.raw_scroll_delta.y,
        )
    });
    let scroll = if dy.abs() >= dx.abs() { dy } else { dx };
    if shift && scroll.abs() > 0.5 {
        // Step by two so the region stays ODD (1×1, 3×3, 5×5, …) and the centre
        // pixel stays centred (owner request); the default region is 1.
        let step = if scroll > 0.0 { 2 } else { -2 };
        app.eyedropper_region =
            (app.eyedropper_region as i32 + step).clamp(1, MAX_REGION as i32) as u32;
    }

    // A press away from the Viewer cancels — but only once primed, so the very
    // click that armed it (landing on another panel) never self-cancels.
    let pressed = ui.ctx().input(|i| i.pointer.any_pressed());
    if app.eyedropper_primed && pressed && !over_viewer {
        app.eyedropper = None;
        return;
    }

    // Show a dropper cursor the whole time the tool is armed, anywhere on screen
    // (owner request UI-9: it used to appear only over the image). Hide the OS
    // pointer and paint the eyedropper glyph, tip at the sample point, on a
    // foreground layer so it reads over any panel — while the magnifier below
    // stays viewfinder-only. egui has no dropper CursorIcon, so this stands in.
    ui.ctx().set_cursor_icon(egui::CursorIcon::None);
    {
        let g = 18.0;
        let icon_rect =
            egui::Rect::from_min_size(cursor + egui::vec2(-1.0, 1.0 - g), egui::vec2(g, g));
        let fg = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("eyedropper-cursor"),
        ));
        crate::icons::paint(
            &fg,
            icon_rect,
            crate::icons::Icon::Eyedropper,
            theme.accent,
            1.4,
        );
    }

    let region = app.eyedropper_region;
    let key = app
        .preview_comp
        .and_then(|comp| app.frame_key_for(comp, app.preview_frame));
    let clicked = over_image && view.clicked();
    // The referenced depth layer's own decoded pixels, stashed by the shell for
    // a depth pick (see `stash_depth_source`). When present, a depth pick reads
    // TRUE depth from this pass at the cursor rather than the composite's luma
    // (a hidden depth layer never shows in the composite). None → the old
    // composite-luma fallback, so a not-yet-decoded reference still gives a value.
    let depth_src: Option<DepthSource> = ui.ctx().data(|d| d.get_temp(depth_pixels_id()));
    // Whether the DoF effect this pick edits inverts its depth: Focus then lives
    // in the inverted space, so the sampled value is inverted once here — the
    // caption preview and the committed value stay in step (commit_sample no
    // longer re-inverts).
    let depth_invert = matches!(target.mode, crate::app_state::EyedropperMode::Depth) && {
        let doc = app.store.snapshot();
        app.preview_comp
            .and_then(|c| doc.comp(c))
            .and_then(|comp| comp.layers.iter().find(|l| l.id == target.layer))
            .filter(|l| target.effect < l.effects.len())
            .and_then(|l| l.effects[target.effect].bool_of("depth_invert"))
            .unwrap_or(false)
    };

    let mut sample: Option<Sample> = None;
    if over_image {
        if let Some(k) = key {
            // Scope the immutable borrow of the frame cache so the commit below
            // can take `app` mutably.
            let drew = {
                if let Some(frame) = app.comp_frame_cache.peek(&k) {
                    let (w, h) = (frame.width as i32, frame.height as i32);
                    if w > 0 && h > 0 && frame.rgba.len() >= (w as usize * h as usize * 4) {
                        let u = ((cursor.x - draw.left()) / draw.width()).clamp(0.0, 1.0);
                        let v = ((cursor.y - draw.top()) / draw.height()).clamp(0.0, 1.0);
                        let cx = ((u * w as f32) as i32).clamp(0, w - 1);
                        let cy = ((v * h as f32) as i32).clamp(0, h - 1);
                        // The depth pass at the cursor: `u,v` index the depth
                        // layer directly (a framing-matched depth pass, K-124).
                        let depth_at = depth_src.as_ref().map(|p| {
                            let (dw, dh) = (p.0 as i32, p.1 as i32);
                            let dcx = ((u * dw as f32) as i32).clamp(0, dw - 1);
                            let dcy = ((v * dh as f32) as i32).clamp(0, dh - 1);
                            let d = average_depth(&p.2, dw, dh, dcx, dcy, region);
                            if depth_invert {
                                1.0 - d
                            } else {
                                d
                            }
                        });
                        draw_magnifier(
                            ui.painter(),
                            theme,
                            &frame.rgba,
                            w,
                            h,
                            cx,
                            cy,
                            region,
                            cursor,
                            image_area,
                            target.mode,
                            depth_at,
                        );
                        if clicked {
                            sample = Some(match target.mode {
                                crate::app_state::EyedropperMode::Colour => Sample::Colour(
                                    average_colour(&frame.rgba, w, h, cx, cy, region),
                                ),
                                // Position pick (T14): the clicked comp pixel's
                                // x/y, written to the x/y parameter pair.
                                crate::app_state::EyedropperMode::Position { .. } => {
                                    Sample::Position(f64::from(cx), f64::from(cy))
                                }
                                crate::app_state::EyedropperMode::Depth => {
                                    Sample::Depth(depth_at.unwrap_or_else(|| {
                                        // Undecoded-reference fallback: still honour
                                        // the invert so it matches the primary path.
                                        let raw = average_depth(&frame.rgba, w, h, cx, cy, region);
                                        if depth_invert {
                                            1.0 - raw
                                        } else {
                                            raw
                                        }
                                    }))
                                }
                            });
                        }
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            };
            if !drew {
                hint(
                    ui.painter(),
                    theme,
                    cursor,
                    "Frame not cached — pause on it",
                );
            }
        } else {
            hint(ui.painter(), theme, cursor, "Open a composition to sample");
        }
    }

    app.eyedropper_primed = true;

    if let Some(s) = sample {
        commit_sample(app, target, s);
        app.eyedropper = None;
        app.refresh_preview();
    }
}

/// A sampled value, ready to write to its target parameter.
#[cfg(feature = "media")]
#[derive(Clone, Copy)]
enum Sample {
    /// Scene-linear RGB for a Colour parameter.
    Colour([f64; 3]),
    /// A 0..1 depth proxy for a Float parameter (DoF Focus).
    Depth(f64),
    /// A comp-pixel position (x, y) for a pair of Float parameters (T14).
    Position(f64, f64),
}

/// Write a sampled value back to its parameter as one undoable
/// `SetLayerEffects`. Silently no-ops if the target has since moved (the layer,
/// effect or parameter index no longer resolves, or its kind changed).
#[cfg(feature = "media")]
fn commit_sample(app: &mut AppState, target: EyedropperTarget, sample: Sample) {
    use lumit_core::anim::Property;
    use lumit_core::model::EffectValue;
    let doc = app.store.snapshot();
    let Some(comp_id) = app.preview_comp else {
        return;
    };
    let Some(comp) = doc.comp(comp_id) else {
        return;
    };
    let Some(layer) = comp.layers.iter().find(|l| l.id == target.layer) else {
        return;
    };
    if target.effect >= layer.effects.len() {
        return;
    }
    let mut effects = layer.effects.clone();
    let params = &mut effects[target.effect].params;
    if target.param >= params.len() {
        return;
    }
    // A position pick (T14) writes TWO Float params: x to `param`, y to the
    // mode's `y_param`. Handled before the single-param match below.
    if let Sample::Position(x, y) = sample {
        let crate::app_state::EyedropperMode::Position { y_param } = target.mode else {
            return;
        };
        if y_param >= params.len() {
            return;
        }
        if !matches!(params[target.param].value, EffectValue::Float(_))
            || !matches!(params[y_param].value, EffectValue::Float(_))
        {
            return;
        }
        params[target.param].value = EffectValue::Float(Property::fixed(x));
        params[y_param].value = EffectValue::Float(Property::fixed(y));
        app.commit(lumit_core::Op::SetLayerEffects {
            comp: comp_id,
            layer: target.layer,
            effects,
        });
        return;
    }
    match (&mut params[target.param].value, sample) {
        (EffectValue::Colour(chs), Sample::Colour(rgb)) => {
            // R, G, B only; the parameter's alpha channel is left untouched.
            chs[0] = Property::fixed(rgb[0]);
            chs[1] = Property::fixed(rgb[1]);
            chs[2] = Property::fixed(rgb[2]);
        }
        (slot @ EffectValue::Float(_), Sample::Depth(d)) => {
            // `d` is already in Focus space (any DoF depth_invert was applied at
            // sample time in viewer_overlay), so store it directly.
            *slot = EffectValue::Float(Property::fixed(d));
        }
        _ => return,
    }
    app.commit(lumit_core::Op::SetLayerEffects {
        comp: comp_id,
        layer: target.layer,
        effects,
    });
}

/// Average the sampled region as scene-linear RGB (the cache holds sRGB bytes;
/// each channel is decoded to linear before averaging, so the committed colour
/// matches the parameter's space). The region is centred on `(cx, cy)`.
#[cfg(feature = "media")]
fn average_colour(rgba: &[u8], w: i32, h: i32, cx: i32, cy: i32, region: u32) -> [f64; 3] {
    let half = (region as i32 - 1) / 2;
    let mut sum = [0.0f64; 3];
    let mut n = 0.0f64;
    for dy in 0..region as i32 {
        for dx in 0..region as i32 {
            let px = (cx - half + dx).clamp(0, w - 1);
            let py = (cy - half + dy).clamp(0, h - 1);
            let i = ((py * w + px) * 4) as usize;
            sum[0] += f64::from(crate::pixels::srgb_decode(rgba[i]));
            sum[1] += f64::from(crate::pixels::srgb_decode(rgba[i + 1]));
            sum[2] += f64::from(crate::pixels::srgb_decode(rgba[i + 2]));
            n += 1.0;
        }
    }
    if n > 0.0 {
        [sum[0] / n, sum[1] / n, sum[2] / n]
    } else {
        [0.0; 3]
    }
}

/// A 0..1 depth value: the Rec.709 luma of the sampled region (linear light),
/// averaged. For the DoF Focus pick this runs on the referenced depth layer's
/// own decoded pixels (stashed by the shell — a grey depth pass has luma equal
/// to its red channel, the value the effect reads), falling back to the
/// composite when that pass has not decoded.
#[cfg(feature = "media")]
fn average_depth(rgba: &[u8], w: i32, h: i32, cx: i32, cy: i32, region: u32) -> f64 {
    let half = (region as i32 - 1) / 2;
    let mut luma = 0.0f64;
    let mut n = 0.0f64;
    for dy in 0..region as i32 {
        for dx in 0..region as i32 {
            let px = (cx - half + dx).clamp(0, w - 1);
            let py = (cy - half + dy).clamp(0, h - 1);
            let i = ((py * w + px) * 4) as usize;
            let r = f64::from(crate::pixels::srgb_decode(rgba[i]));
            let g = f64::from(crate::pixels::srgb_decode(rgba[i + 1]));
            let b = f64::from(crate::pixels::srgb_decode(rgba[i + 2]));
            luma += 0.2126 * r + 0.7152 * g + 0.0722 * b;
            n += 1.0;
        }
    }
    if n > 0.0 {
        (luma / n).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// The magnifier panel: a zoomed 9×9 grid of the pixels around the cursor,
/// dotted grid lines between cells, the averaged region ringed by a solid
/// theme-rounded border, and a caption showing the region size and the value
/// that would be committed. All colours and strokes come from `theme`.
#[cfg(feature = "media")]
#[allow(clippy::too_many_arguments)]
fn draw_magnifier(
    painter: &egui::Painter,
    theme: &Theme,
    rgba: &[u8],
    w: i32,
    h: i32,
    cx: i32,
    cy: i32,
    region: u32,
    cursor: egui::Pos2,
    bounds: egui::Rect,
    mode: crate::app_state::EyedropperMode,
    // For a depth pick, the depth sampled from the referenced depth layer at the
    // cursor (see `stash_depth_source`); the caption previews this instead of the
    // composite's luma. None falls back to the composite (undecoded reference).
    depth_override: Option<f64>,
) {
    const N: i32 = 9;
    const HALF: i32 = N / 2;
    const CELL: f32 = 13.0;
    let pad = 6.0f32;
    let cap_h = 18.0f32;
    let grid = CELL * N as f32;
    let panel_w = grid + pad * 2.0;
    let panel_h = grid + pad * 2.0 + cap_h;

    // Follow the cursor, clamped to stay within the Viewer image area.
    let mut min = cursor + egui::vec2(18.0, 18.0);
    min.x = min
        .x
        .clamp(bounds.left(), (bounds.right() - panel_w).max(bounds.left()));
    min.y = min
        .y
        .clamp(bounds.top(), (bounds.bottom() - panel_h).max(bounds.top()));
    let panel = egui::Rect::from_min_size(min, egui::vec2(panel_w, panel_h));

    // Corner rounding follows the theme's card corner token: a sharp square
    // under the Sharp shape, rounded under Round (no hardcoded shape flag). The
    // fill goes down first; the hairline border is drawn LAST, over the preview,
    // so the zoomed pixels sit BEHIND it instead of spilling over the rounded
    // edge (owner UI-15).
    let round = f32::from(theme.tokens.card_radius);
    painter.rect_filled(panel, round, theme.surface_3);

    // Everything inside the viewfinder paints through a painter clipped to just
    // inside the border, so the preview can never cross it (owner UI-15).
    let inner = painter.with_clip_rect(panel.shrink(1.0));

    let grid_min = panel.min + egui::vec2(pad, pad);
    // Round the grid's four outer corner cells to match the panel's corner (owner
    // request), concentric with the panel radius less the padding — so under the
    // Round shape the pixel square's corners curve like the box, and under Sharp
    // (radius 0) they stay square. Adaptable to any future radius token.
    let outer = f32::from(theme.tokens.card_radius);
    let corner = (outer - pad).clamp(0.0, CELL) as u8;
    for gy in 0..N {
        for gx in 0..N {
            let px = (cx - HALF + gx).clamp(0, w - 1);
            let py = (cy - HALF + gy).clamp(0, h - 1);
            let i = ((py * w + px) * 4) as usize;
            let col = crate::theme::document_colour([rgba[i], rgba[i + 1], rgba[i + 2], 255]);
            let cell = egui::Rect::from_min_size(
                grid_min + egui::vec2(gx as f32 * CELL, gy as f32 * CELL),
                egui::vec2(CELL, CELL),
            );
            let mut round = egui::CornerRadius::ZERO;
            if gx == 0 && gy == 0 {
                round.nw = corner;
            }
            if gx == N - 1 && gy == 0 {
                round.ne = corner;
            }
            if gx == 0 && gy == N - 1 {
                round.sw = corner;
            }
            if gx == N - 1 && gy == N - 1 {
                round.se = corner;
            }
            inner.rect_filled(cell, round, col);
        }
    }

    // Dotted grid lines between the cells.
    for k in 1..N {
        let x = grid_min.x + k as f32 * CELL;
        dotted_segment(
            &inner,
            egui::pos2(x, grid_min.y),
            egui::pos2(x, grid_min.y + grid),
            theme.hairline,
        );
        let y = grid_min.y + k as f32 * CELL;
        dotted_segment(
            &inner,
            egui::pos2(grid_min.x, y),
            egui::pos2(grid_min.x + grid, y),
            theme.hairline,
        );
    }

    // The averaged region: a solid, theme-rounded border around the centre
    // block (just the centre cell at 1×1, growing with Shift+scroll).
    let r = region as i32;
    let left = HALF - (r - 1) / 2;
    let block = egui::Rect::from_min_size(
        grid_min + egui::vec2(left as f32 * CELL, left as f32 * CELL),
        egui::vec2(r as f32 * CELL, r as f32 * CELL),
    );
    let cell_round = f32::from(theme.tokens.control_radius);
    inner.rect_stroke(
        block,
        cell_round,
        egui::Stroke::new(1.6_f32, theme.accent),
        egui::StrokeKind::Inside,
    );

    // The bottom info bar: a strip under the grid carrying the swatch and the
    // region size. It takes the card-corner token, which egui clamps to a pill at
    // the Round radius (a rounded strip spanning the bottom) and leaves a square
    // bar under Sharp — no hardcoded shape flag (owner UI-15).
    let bar = egui::Rect::from_min_max(
        egui::pos2(grid_min.x, grid_min.y + grid + pad * 0.5),
        egui::pos2(grid_min.x + grid, panel.bottom() - pad * 0.5),
    );
    inner.rect_filled(bar, round, theme.surface_2);

    // Caption: a swatch of the value that would be committed, then the size. It
    // starts far enough in to clear the pill's rounded left end.
    let cap_y = bar.center().y;
    let sw =
        egui::Rect::from_center_size(egui::pos2(bar.left() + 10.0, cap_y), egui::vec2(10.0, 10.0));
    let swatch_col = match mode {
        // A position pick (T14) shows the pixel under the cursor, like a colour
        // pick — a visual anchor for where the x/y will land.
        crate::app_state::EyedropperMode::Colour
        | crate::app_state::EyedropperMode::Position { .. } => {
            let avg = average_colour(rgba, w, h, cx, cy, region);
            crate::theme::document_colour([
                crate::pixels::srgb_encode(avg[0] as f32),
                crate::pixels::srgb_encode(avg[1] as f32),
                crate::pixels::srgb_encode(avg[2] as f32),
                255,
            ])
        }
        crate::app_state::EyedropperMode::Depth => {
            let d = depth_override.unwrap_or_else(|| average_depth(rgba, w, h, cx, cy, region));
            let g = crate::pixels::srgb_encode(d as f32);
            crate::theme::document_colour([g, g, g, 255])
        }
    };
    inner.rect_filled(sw, cell_round, swatch_col);
    inner.text(
        egui::pos2(sw.right() + 6.0, cap_y),
        egui::Align2::LEFT_CENTER,
        format!("{r}×{r}"),
        egui::FontId::proportional(11.0),
        theme.text_secondary,
    );

    // The hairline border LAST, on top of the preview and info bar, so the
    // zoomed pixels sit behind it at the rounded edge (owner UI-15).
    painter.rect_stroke(
        panel,
        round,
        egui::Stroke::new(1.0_f32, theme.hairline_strong),
        egui::StrokeKind::Inside,
    );
}

/// A dotted line from `a` to `b`, drawn as small theme-coloured dots.
#[cfg(feature = "media")]
fn dotted_segment(painter: &egui::Painter, a: egui::Pos2, b: egui::Pos2, color: egui::Color32) {
    let span = b - a;
    let len = span.length();
    if len < 0.5 {
        return;
    }
    let dir = span / len;
    let step = 3.0;
    let mut t = 0.0;
    while t <= len {
        painter.circle_filled(a + dir * t, 0.6, color);
        t += step;
    }
}

/// A small themed tooltip near the cursor (shown when there is no frame to
/// sample yet).
#[cfg(feature = "media")]
fn hint(painter: &egui::Painter, theme: &Theme, cursor: egui::Pos2, text: &str) {
    let pos = cursor + egui::vec2(18.0, 18.0);
    let galley = painter.layout_no_wrap(
        text.to_owned(),
        egui::FontId::proportional(11.0),
        theme.text_muted,
    );
    let rect = egui::Rect::from_min_size(pos, galley.size() + egui::vec2(10.0, 6.0));
    painter.rect_filled(
        rect,
        f32::from(theme.tokens.control_radius),
        theme.surface_3,
    );
    painter.galley(pos + egui::vec2(5.0, 3.0), galley, theme.text_muted);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// An arm request survives a round-trip through context data and is drained
    /// exactly once — the channel the inspector button uses to reach the shell.
    #[test]
    fn arm_request_round_trips_and_drains_once() {
        let ctx = egui::Context::default();
        assert!(take_arm_request(&ctx).is_none());
        let target = EyedropperTarget {
            layer: uuid::Uuid::now_v7(),
            effect: 1,
            param: 2,
            mode: crate::app_state::EyedropperMode::Colour,
        };
        request_arm(&ctx, target);
        assert_eq!(take_arm_request(&ctx), Some(target));
        // Drained: a second take yields nothing.
        assert!(take_arm_request(&ctx).is_none());
    }

    /// A single-pixel region lifts that exact pixel, decoded to scene-linear:
    /// an sRGB 255 is linear 1.0, an sRGB 0 is linear 0.0.
    #[cfg(feature = "media")]
    #[test]
    fn average_colour_region_one_is_the_single_linear_pixel() {
        // 2×2, top-left = pure red.
        let rgba = vec![
            255, 0, 0, 255, 0, 0, 0, 255, // row 0
            0, 0, 0, 255, 0, 0, 0, 255, // row 1
        ];
        let c = average_colour(&rgba, 2, 2, 0, 0, 1);
        assert!((c[0] - 1.0).abs() < 1e-6, "{c:?}");
        assert!(c[1].abs() < 1e-6, "{c:?}");
        assert!(c[2].abs() < 1e-6, "{c:?}");
    }

    /// A wider region averages in linear light: one white pixel among three
    /// black ones lands at 0.25 per channel, not the sRGB midpoint.
    #[cfg(feature = "media")]
    #[test]
    fn average_colour_region_averages_in_linear() {
        // 2×2, top-left white, the rest black. Region 2 centred at (0,0)
        // (half = 0) covers all four pixels.
        let rgba = vec![
            255, 255, 255, 255, 0, 0, 0, 255, // row 0
            0, 0, 0, 255, 0, 0, 0, 255, // row 1
        ];
        let c = average_colour(&rgba, 2, 2, 0, 0, 2);
        for ch in c {
            assert!((ch - 0.25).abs() < 1e-6, "{c:?}");
        }
    }

    /// The depth proxy is Rec. 709 luma in linear light: white is 1.0, black
    /// is 0.0, and the result is clamped to 0..1.
    #[cfg(feature = "media")]
    #[test]
    fn average_depth_is_clamped_luma() {
        assert!((average_depth(&[255, 255, 255, 255], 1, 1, 0, 0, 1) - 1.0).abs() < 1e-6);
        assert!(average_depth(&[0, 0, 0, 255], 1, 1, 0, 0, 1).abs() < 1e-6);
    }
}
