//! Pictures of one thing on its own: the footage view, the layer view and
//! the angle viewer (docs/impl/multi-viewer.md §3.6).

use super::*;

/// Draw one item on its own: the footage view and the layer view
/// (docs/impl/multi-viewer.md §3.6).
///
/// **No second render path.** A scratch composition is built around the item
/// on a clone of the document and composited by the ordinary walk, so the
/// picture arrives down the same zero-copy transport at the same quality, and
/// the frame names itself the way every other frame does. Nothing is
/// committed: the composition never reaches the document, the journal, the
/// undo stack or the Project panel.
#[frb(ignore)]
pub(super) fn render_item(
    req: RenderItemRequest,
    state: &mut WorkerState,
    stream: &mut WorkerResponseStream,
) -> Result<(), BridgeError> {
    // The angle viewer is drawn beside the program while it plays, at the
    // page, size and scale it last asked for. Another picture for its view
    // means it shows something else now.
    state.angles = match req.of {
        crate::scratch::ScratchOf::Angles { comp, page } => Some(AnglesLatch {
            view: req.view,
            comp,
            page,
            size: req.size,
            scale: req.scale,
            kept: None,
            asked_to: None,
            shown: None,
            drawn: None,
            cost: None,
            noted: (web_time::Instant::now(), 0),
        }),
        _ => state.angles.take().filter(|latch| latch.view != req.view),
    };
    let document = {
        let document = req.project.state()?;
        let document = document.read().map_err(|_| BridgeError::ReadFailed)?;
        document.store.snapshot()
    };
    // A nonsense rate reads as 25, which is a rate rather than a division by
    // zero: a view of a still needs one only to have a clock at all.
    let Ok(rate) = lumit_core::time::FrameRate::new(req.rate.0, req.rate.1)
        .or_else(|_| lumit_core::time::FrameRate::new(25, 1))
    else {
        return Ok(());
    };
    // The item's own length, as a time: frames over the rate, exactly.
    let Ok(duration) = lumit_core::time::Rational::from_i128(
        i128::from(req.frames.max(1)) * i128::from(req.rate.1.max(1)),
        i128::from(req.rate.0.max(1)),
    ) else {
        return Ok(());
    };
    let duration = lumit_core::time::Duration(duration);
    let Some(document) =
        crate::scratch::document_with(&document, req.of, req.effects, req.size, rate, duration)
    else {
        // The item has gone, or the layer has. The view shows its empty state
        // rather than a picture of something else.
        return Ok(());
    };
    state.set_view_id(req.view);
    look_at(state, req.view);
    // Not `last_shown`: the fill, the cache bar and the drags follow the
    // active composition view, and this item's scratch comp is none of those.
    let comp = req.of.comp_id();
    watched(state, stream, req.frame, |state, stream| {
        publish_frame(
            state,
            comp,
            req.frame,
            req.scale,
            &document,
            stream,
            BridgePlaybackMode::Adaptive,
            // A scratch composition is not the document, so its frames are not
            // banked against one: cacheable would file a picture under a
            // composition nothing else can ever ask for.
            false,
        );
    });
    Ok(())
}

/// The angle viewer as it last asked to be drawn, so playback can draw it
/// beside each picture of the program.
#[frb(ignore)]
pub(super) struct AnglesLatch {
    pub(super) view: u32,
    /// The multicam comp it shows.
    comp: Uuid,
    page: u32,
    size: (u32, u32),
    scale: f32,
    /// The grid composition last built, with the document revision and the
    /// multicam comp it was built for: one copy of the document a run, not
    /// one a frame.
    kept: Option<(u64, Uuid, std::sync::Arc<lumit_core::model::Document>)>,
    /// The last frame of the multicam comp whose decodes were asked for.
    asked_to: Option<(Uuid, i64)>,
    /// The frame of the multicam comp the last grid showed. A grid of an
    /// earlier moment, or of another comp, forgets [`Self::asked_to`], so a
    /// loop or a jump back asks for its decodes again.
    shown: Option<(Uuid, i64)>,
    /// When the grid was last drawn beside the program.
    drawn: Option<web_time::Instant>,
    /// What the last grid took to draw, in seconds.
    cost: Option<f64>,
    /// When the grid's pace was last noted, and the grids drawn since.
    noted: (web_time::Instant, u32),
}

/// The grid is drawn beside the program at most this often, so the program
/// keeps its own rate: every angle's picture is decoded and uploaded for
/// each grid.
const GRID_EVERY: std::time::Duration = std::time::Duration::from_millis(30);

/// A grid too dear to fit between two pictures of the program is still drawn
/// this often, so the angles never stand still beside a program that moves.
const GRID_AT_LEAST: std::time::Duration = std::time::Duration::from_millis(500);

/// While the program plays over a multicam clip, draw the angle viewer at the
/// moment of the multicam comp the program shows at `frame`, and ask for the
/// next few moments' decodes so the next grid finds them ready. Nothing while
/// the angle viewer is closed or the program is over no multicam clip, and no
/// more often than [`GRID_EVERY`]. `room` is the seconds the program can
/// spare, `None` when it has none to give: a grid that took longer than that
/// last time waits, up to [`GRID_AT_LEAST`]. The program comes first.
#[frb(ignore)]
pub(super) fn follow(
    state: &mut WorkerState,
    stream: &mut WorkerResponseStream,
    program: Uuid,
    frame: u64,
    room: Option<f64>,
) {
    let Some(latch) = state.angles.as_ref() else {
        return;
    };
    let since = latch.drawn.map(|d| d.elapsed());
    let fits = room.is_some_and(|room| latch.cost.is_none_or(|cost| cost <= room));
    if since.is_some_and(|s| s < GRID_EVERY || (!fits && s < GRID_AT_LEAST)) {
        return;
    }
    let started = web_time::Instant::now();
    let Ok(project) = state.project.state() else {
        return;
    };
    let Ok((document, revision)) = project.read().map(|p| p.store.snapshot_and_revision()) else {
        return;
    };
    drop(project);
    let Some(comp) = document.comp(program) else {
        return;
    };
    let at = |f: u64| -> Option<(Uuid, i64)> {
        let t = comp.frame_rate.time_of_frame(i64::try_from(f).ok()?).ok()?;
        let under = lumit_core::multicam::under(&document, comp, t.0)?;
        let mc = document.comp(under.comp)?;
        Some((
            under.comp,
            mc.frame_rate
                .frame_at(lumit_core::time::CompTime(under.time)),
        ))
    };
    let Some((mc_id, mc_frame)) = at(frame) else {
        return;
    };
    let Some(mc) = document.comp(mc_id) else {
        return;
    };
    let (rate, duration) = (mc.frame_rate, mc.duration);
    let Some(latch) = state.angles.as_mut() else {
        return;
    };
    latch.comp = mc_id;
    if latch.shown.is_some_and(|(c, f)| c != mc_id || mc_frame < f) {
        latch.asked_to = None;
    }
    latch.shown = Some((mc_id, mc_frame));
    let id = crate::scratch::ScratchOf::Angles {
        comp: mc_id,
        page: latch.page,
    }
    .comp_id();
    let grid = match &latch.kept {
        Some((r, c, kept)) if *r == revision && *c == mc_id => kept.clone(),
        _ => {
            let Some(made) = crate::api::multicam::grid::document(
                &document, id, mc_id, latch.page, latch.size, rate, duration,
            ) else {
                return;
            };
            latch.kept = Some((revision, mc_id, made.clone()));
            made
        }
    };
    let (view, scale, asked) = (latch.view, latch.scale, latch.asked_to);
    let quality = crate::render::quality_for(scale);
    let mut last = None;
    for k in 1..=crate::playback::PREFETCH_AHEAD {
        let Some((c, f)) = at(frame + k) else {
            continue;
        };
        if c != mc_id || asked.is_some_and(|(ac, af)| ac == c && f <= af) {
            continue;
        }
        let wants =
            state
                .renderer
                .prefetch_wants(&grid, id, u64::try_from(f).unwrap_or(0), quality);
        state.prefetcher.request(wants);
        last = Some((c, f));
    }
    if let Some(latch) = state.angles.as_mut() {
        latch.asked_to = last.or(latch.asked_to);
        latch.drawn = Some(web_time::Instant::now());
    }
    let playing = state.view_id();
    state.set_view_id(view);
    look_at(state, view);
    publish_frame(
        state,
        id,
        u64::try_from(mc_frame).unwrap_or(0),
        scale,
        &grid,
        stream,
        BridgePlaybackMode::Adaptive,
        false,
    );
    state.set_view_id(playing);
    look_at(state, playing);
    if let Some(latch) = state.angles.as_mut() {
        let cost = started.elapsed().as_secs_f64();
        latch.cost = Some(cost);
        latch.noted.1 += 1;
        // The sparse pace line, as the program has its own.
        let since = latch.noted.0.elapsed().as_secs_f64();
        if since >= 2.0 {
            note!(
                "Angle viewer pace: {:.1} grids a second, the last in {:.2} ms",
                f64::from(latch.noted.1) / since,
                cost * 1000.0,
            );
            latch.noted = (web_time::Instant::now(), 0);
        }
    }
}
