//! Where Clone to points puts its copies, and which picture each one shows.

use super::*;

impl CloneToPoints {
    /// The raster factor, for the one input the declaration cannot scale: a
    /// stream read off a wire is in px@comp, like a mask path, and has to be
    /// rearranged into the pixels the frame is drawn at.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// How many layer rows are set, which the bag does not otherwise say.
    pub const DERIVED_LAYERS: ParamId = ParamId::new("derived.layers");

    /// Whether the copies are rendered one per clone.
    pub const DERIVED_PER_CLONE: ParamId = ParamId::new("derived.per_clone");

    /// Time step as the builder read it, off its own keyframes.
    pub const DERIVED_TIME_STEP: ParamId = ParamId::new("derived.time_step");

    /// This instance's raster factor, read back out of a resolved bag.
    #[must_use]
    pub fn px_scale_of(p: Params<'_>) -> f32 {
        p.float(Self::DERIVED_PX_SCALE, 1.0)
    }

    /// The stream this effect actually stamps: the wired one, with the two
    /// dials and the cap applied.
    ///
    /// **Every decision is here**, in one expression both render paths read, so
    /// the CPU oracle and the instanced draw cannot come to stamp different
    /// squares. `in_stream` is in the units the caller wants out — px@comp for
    /// a reader, raster pixels for a draw ([`PointsStream::rescaled`]).
    ///
    /// `pictures` is how the pictures to choose from are laid out. Which one
    /// each stamp takes is left in the answer's `index` column, as
    /// [`picture_of`](Self::picture_of) chose it.
    #[must_use]
    pub fn stamps(self, in_stream: &PointsStream, pictures: Pictures) -> PointsStream {
        let mut out = in_stream.clone();
        // Chosen before anything is dropped or reordered, so a point keeps
        // its picture when the camera moves or a neighbour is left out.
        let numbers = self.numbers(in_stream);
        out.index = (0..in_stream.len())
            .map(|i| self.picture_at(in_stream, i, pictures, numbers).first as f32)
            .collect();
        // How many pictures deep every point is stamped, and how far apart
        // they sit in the list. The same for every point.
        let Stamped { step, count, .. } = self.picture_of(in_stream, 0, pictures);
        if self.apply_to != 0 {
            let group = in_stream.group(self.apply_group.as_str());
            out.retain(|i| in_stream.applies(self.apply_to, group, self.apply_threshold, i));
        }
        // The newest by birth index, which is the cap rule the whole family
        // applies — deterministic, and the same from any scrub direction.
        // Max clones counts stamps, so a point stamped several deep counts
        // that many times and keeps all its stamps or none.
        let most = self.max_clones.clamp(0, points::CAP_HARD as i32) as usize;
        out.keep_newest(most / count);
        // Scale, Scale x / y, Rotation, Offset x / y, Opacity and Tint, per
        // stamp where a driver that reads the copy is wired into the row, and
        // the row's one value elsewhere.
        let scales = per_copy::column_or(&out, per_copy::COPY_SCALE, self.scale);
        let wides = per_copy::column_or(&out, per_copy::COPY_SCALE_X, self.scale_x);
        let talls = per_copy::column_or(&out, per_copy::COPY_SCALE_Y, self.scale_y);
        let turns = per_copy::column_or(&out, per_copy::COPY_ROTATION, self.rotation);
        let opacities = per_copy::column_or(&out, per_copy::COPY_OPACITY, self.opacity);
        let tints = per_copy::column_or(&out, per_copy::COPY_TINT, self.tint);
        // A driver hands out px@comp, while the row is already in the
        // stream's units: only a driven column is scaled.
        let units = out.units();
        let along = |name: &str, row: f32| -> Vec<f32> {
            match out.column(name) {
                points::Column::Named(_) => per_copy::column_or(&out, name, 0.0)
                    .into_iter()
                    .map(|v| v * units)
                    .collect(),
                _ => vec![row; out.len()],
            }
        };
        let dxs = along(per_copy::COPY_OFFSET_X, self.offset_x);
        let dys = along(per_copy::COPY_OFFSET_Y, self.offset_y);
        for (s, scale) in out.size.iter_mut().zip(&scales) {
            *s *= (scale / 100.0).max(0.0);
        }
        // Across and down in the stamp's own turned frame, on top of the
        // point's own stretch. Left empty while nothing is stretched.
        if wides.iter().chain(&talls).any(|s| *s != 100.0) {
            for ((st, w), t) in out.stretch_mut().iter_mut().zip(&wides).zip(&talls) {
                st[0] *= (w / 100.0).max(0.0);
                st[1] *= (t / 100.0).max(0.0);
            }
        }
        for ((p, dx), dy) in out.position.iter_mut().zip(&dxs).zip(&dys) {
            p[0] += dx;
            p[1] += dy;
        }
        for (r, turn) in out.rotation.iter_mut().zip(&turns) {
            *r += turn.to_radians();
        }
        for ((c, opacity), tint) in out.colour.iter_mut().zip(&opacities).zip(&tints) {
            let opacity = (opacity / 100.0).clamp(0.0, 1.0);
            let tint = (tint / 100.0).clamp(0.0, 1.0);
            // Towards opaque white, which is the identity of a premultiplied
            // tint: at Tint 0 a stamp is the layer's own picture, at 100 it is
            // the picture times the point's colour and alpha.
            for ch in c.iter_mut() {
                *ch = (1.0 + (*ch - 1.0) * tint) * opacity;
            }
        }
        if self.depth_sort {
            out.sort_far_to_near();
        }
        // Every layer: each point becomes `count` stamps, one after the
        // other, after the sort so the order stays one order. Made with the
        // stream's own append and sort, so no column can be left behind:
        // each copy is numbered by its point's place, and the sort by that
        // number keeps the copies in layer order.
        if count > 1 {
            let places = out.len();
            let ids = std::mem::replace(&mut out.id, (0..places as u64).collect());
            let one = out.clone();
            for j in 1..count {
                let mut next = one.clone();
                for picture in &mut next.index {
                    *picture += (j * step) as f32;
                }
                out.append(&next);
            }
            out.sort_by_id();
            out.id = (0..places * count)
                .filter_map(|k| ids.get(k / count).copied())
                .collect();
        }
        out
    }

    /// The pictures point `i` of the wired stream is stamped with, as places
    /// in the list the render hands this effect.
    ///
    /// The one place a point's pictures are chosen. Choose by picks the
    /// layer, or the clone where the copies are rendered one per clone, and
    /// Time offset picks the moment. Every layer takes them all, bottom
    /// first, each at the point's own moment.
    #[must_use]
    pub fn picture_of(self, s: &PointsStream, i: usize, of: Pictures) -> Stamped {
        self.picture_at(s, i, of, self.numbers(s))
    }

    /// The numbers Choose by and Time offset read under Number, found on
    /// `s` once for a walk over its points.
    fn numbers(self, s: &PointsStream) -> [Column; 2] {
        [
            s.column(self.choose_name.as_str()),
            s.column(self.time_name.as_str()),
        ]
    }

    /// [`picture_of`](Self::picture_of), with the two numbers found already.
    fn picture_at(self, s: &PointsStream, i: usize, of: Pictures, numbers: [Column; 2]) -> Stamped {
        let [choose, time] = numbers;
        let one = |first: usize| Stamped {
            first,
            step: 1,
            count: 1,
        };
        let every = self.choose_by == CHOOSE_EVERY;
        let id = s.id.get(i).copied().unwrap_or(0);
        // Which of `n` the point takes, as Choose by says.
        let chosen = |n: usize| -> usize {
            if n < 2 {
                return 0;
            }
            match self.choose_by {
                // Its own dice, from its id, so the same picture every frame.
                1 => {
                    let roll = points::draw(self.seed, id, PICTURE_ATTR);
                    ((roll * n as f32) as usize).min(n - 1)
                }
                // The number it carries, wrapped round the list. A NaN reads 0.
                2 => {
                    let number = s.read(choose, i).unwrap_or(0.0);
                    (number.floor() as i64).rem_euclid(n as i64) as usize
                }
                // The Layer index row, or the copy's own where a driver is
                // wired into it.
                CHOOSE_INDEX => {
                    let column = s.column(per_copy::COPY_LAYER_INDEX);
                    let number = s.read(column, i).unwrap_or(self.layer_index);
                    let number = if number.is_finite() { number } else { 0.0 };
                    (number.floor() as i64).rem_euclid(n as i64) as usize
                }
                _ => i % n,
            }
        };
        if of.renders > 0 {
            if every {
                // The renders come in groups of one a layer, and the points
                // take the groups in turn.
                let deep = of.every_deep();
                return Stamped {
                    first: (i % (of.renders / deep).max(1)) * deep,
                    step: 1,
                    count: deep,
                };
            }
            return one(chosen(of.renders));
        }
        let age = s.age.get(i).copied().unwrap_or(0.0);
        let number = s.read(time, i).unwrap_or(0.0);
        let moment = self.moment_of(i, id, number, age, of.moments);
        if every {
            return Stamped {
                first: moment,
                step: of.moments.max(1),
                count: of.layers.max(1),
            };
        }
        one(chosen(of.layers) * of.moments + moment)
    }

    /// Which of `moments` moments a stamp shows, as Time offset says. `place`
    /// is where it comes in the stream, `number` the number it carries.
    fn moment_of(self, place: usize, id: u64, number: f32, age: f32, moments: usize) -> usize {
        if moments < 2 {
            return 0;
        }
        match self.time_offset {
            // Wrapped round the moments, so the builder need not know how
            // many points there are.
            TIME_IN_ORDER => place % moments,
            TIME_RANDOM => {
                let roll = points::draw(self.seed, id, MOMENT_ATTR);
                ((roll * moments as f32) as usize).min(moments - 1)
            }
            TIME_NUMBER => (number.floor() as i64).rem_euclid(moments as i64) as usize,
            // A point older than every moment holds the last. A NaN reads 0.
            TIME_POINT_AGE if self.time_step > 0.0 => {
                ((age.max(0.0) / self.time_step) as usize).min(moments - 1)
            }
            _ => 0,
        }
    }

    /// The pictures the render makes for this effect. **The cap rule**, in
    /// one place: never more than [`MAX_PICTURES`].
    ///
    /// `layers` is how many layer rows are set, `per_clone` what
    /// [`per_clone_in`](Self::per_clone_in) answered, and `points` how many
    /// points the wire brings where that is known before the render.
    ///
    /// Layers come first. Every set layer keeps a picture and Time samples
    /// gives way, so four layers get eight moments each. Rendered one per
    /// clone there is a render a point up to Max renders, each at its own
    /// clone's moment, and layers past the number of renders are not shown.
    #[must_use]
    pub fn pictures(self, layers: usize, per_clone: bool, points: Option<usize>) -> Pictures {
        let layers = layers.min(MAX_PICTURES);
        if layers == 0 {
            return Pictures::plain(0);
        }
        let most = MAX_PICTURES as i32;
        let samples = match self.time_offset {
            TIME_OFF => 1,
            _ => self.time_samples.clamp(1, most) as usize,
        };
        if per_clone {
            let cap = self.max_renders.clamp(1, most) as usize;
            // Every layer wants a render for each layer of each point.
            let each = match self.choose_by {
                CHOOSE_EVERY => layers,
                _ => 1,
            };
            // A stream that cannot be counted yet gets the whole cap.
            let mut renders = points.map_or(cap, |n| n.saturating_mul(each).clamp(1, cap));
            // Whole points only, so no render is made that no stamp shows.
            if each > 1 && renders > each {
                renders -= renders % each;
            }
            Pictures {
                layers,
                moments: samples,
                renders,
            }
        } else {
            Pictures {
                layers,
                moments: samples.min(MAX_PICTURES / layers).max(1),
                renders: 0,
            }
        }
    }

    /// What picture `k` of the list is.
    #[must_use]
    pub fn picture(self, of: Pictures, k: usize) -> Picture {
        if of.renders == 0 {
            let moments = of.moments.max(1);
            return Picture {
                layer: k / moments,
                moment: k % moments,
                clone: None,
            };
        }
        // A clone's moment is its own number's, so each has both. Every
        // layer stamps one point with a whole group of renders, and they
        // share that point's moment.
        let place = match self.choose_by {
            CHOOSE_EVERY => k / of.every_deep(),
            _ => k,
        };
        Picture {
            layer: k % of.layers.max(1),
            moment: self.moment_of(place, place as u64, place as f32, 0.0, of.moments),
            clone: Some((k as u32, of.renders as u32)),
        }
    }

    /// The composition time a layer is rendered at for moment `m`. `t` is the
    /// frame's own time and `start` the layer's in point.
    #[must_use]
    pub fn moment_time(self, m: usize, t: f64, start: f64) -> f64 {
        let step = m as f64 * f64::from(self.time_step);
        match self.time_offset {
            TIME_POINT_AGE => start + step.max(0.0),
            // The frame's own time to the bit, which needs no second render.
            _ if m == 0 => t,
            _ => t - step,
        }
    }

    /// The rows that decide which pictures are rendered, read off the stored
    /// instance at layer time `lt`. Every other row is at its default.
    ///
    /// The planner, the builder and the draw all read them through here, so
    /// they agree. A wire into Time step, Time samples or Max renders is not
    /// followed: they keep to their own keyframes, as Trail's Samples does.
    #[must_use]
    pub fn stored(e: &EffectInstance, lt: f64) -> Self {
        let mut c = Self::read(Params::EMPTY);
        let choice = |id: &str, or: u32| match e.param(id) {
            Some(EffectValue::Choice(v)) => *v,
            _ => or,
        };
        let number = |id: &str| e.float_at(id, lt).filter(|v| v.is_finite());
        c.choose_by = choice("choose_by", c.choose_by);
        c.time_offset = choice("time_offset", c.time_offset);
        c.per_clone = choice("per_clone", c.per_clone);
        if let Some(EffectValue::Seed(seed)) = e.param("seed") {
            c.seed = *seed;
        }
        c.time_step = number("time_step").map_or(c.time_step, |v| v as f32);
        c.time_samples = number("time_samples").map_or(c.time_samples, |v| v as i32);
        c.max_renders = number("max_renders").map_or(c.max_renders, |v| v as i32);
        c
    }

    /// The points Grid, Path and Layout make at layer time `lt`, px@comp, as a
    /// wire would bring them. `None` for Points input, which reads the wire.
    ///
    /// Grid and Path are Grid's and Points along path's own streams, read
    /// with these rows, so the copies sit where those producers would put
    /// them. Layout's copies all start at Position: where each one goes
    /// depends on how big the copies before it are, which only the draw
    /// knows ([`laid_out`](Self::laid_out)). The rows read their keys and
    /// expressions here, not a wire.
    #[must_use]
    pub fn distributed(
        e: &EffectInstance,
        comp: &Composition,
        owner: &Layer,
        lt: f64,
        context: &Arc<ExpressionContext>,
        projection: points::Projection,
    ) -> Option<PointsStream> {
        let distribution = match e.param("distribution") {
            Some(EffectValue::Choice(v)) => *v,
            _ => DIST_INPUT,
        };
        if distribution == DIST_INPUT {
            return None;
        }
        let d = Self::read(Params::EMPTY);
        let number = |id: &str, or: f32| {
            e.float_at_with_context(id, lt, Arc::clone(context))
                .filter(|v| v.is_finite())
                .map_or(or, |v| v as f32)
        };
        let count = number("dist_count", d.dist_count as f32) as i32;
        let at = [
            number("dist_position_x", d.dist_position_x),
            number("dist_position_y", d.dist_position_y),
        ];
        // A copy of size 100 is the layer at its own size, or a cell, or a
        // square of 100 px: Size says which.
        let size = 100.0;
        let mut stream = match distribution {
            DIST_GRID => crate::fx::effects::grid::Grid {
                columns: number("dist_columns", d.dist_columns as f32) as i32,
                rows: number("dist_rows", d.dist_rows as f32) as i32,
                spacing_x: number("dist_spacing_x", d.dist_spacing_x),
                spacing_y: number("dist_spacing_y", d.dist_spacing_y),
                position_x: at[0],
                position_y: at[1],
                size,
                ..crate::fx::effects::grid::Grid::read(Params::EMPTY)
            }
            .stream(projection),
            DIST_PATH => {
                let target = e
                    .layer_ref(PATH_LAYER)
                    .and_then(|id| comp.layers.iter().find(|l| l.id == id))
                    .unwrap_or(owner);
                let t = lt + owner.start_offset.0.to_f64();
                let path = points::layer_path(comp, owner, target, t, context);
                let align = !matches!(e.param("dist_align"), Some(EffectValue::Bool(false)));
                let mut along = crate::fx::effects::points_along_path::PointsAlongPath {
                    count,
                    align,
                    // Round a closed path the copies are a Length / Count
                    // apart however much of it is used, as Cavalry spaces
                    // them; Points along path counts one gap fewer short of
                    // the whole path.
                    end: spread_end(number("dist_length", d.dist_length), count, path.closed),
                    offset: number("dist_travel", d.dist_travel),
                    wrap: true,
                    size,
                    ..crate::fx::effects::points_along_path::PointsAlongPath::read(Params::EMPTY)
                }
                .stream(&path, projection);
                push_off_path(
                    &mut along,
                    &path,
                    number("dist_offset", d.dist_offset),
                    align,
                );
                along
            }
            _ => {
                let n = count.clamp(0, points::CAP_HARD as i32) as usize;
                let mut out = PointsStream {
                    projection,
                    ..PointsStream::default()
                };
                for k in 0..n {
                    out.position.push([at[0], at[1], 0.0]);
                    out.speed.push([0.0; 3]);
                    out.age.push(0.0);
                    out.life.push(1.0);
                    out.size.push(size);
                    out.rotation.push(0.0);
                    out.colour.push([1.0; 4]);
                    out.id.push(k as u64);
                    out.index.push(if n > 1 {
                        k as f32 / (n - 1) as f32
                    } else {
                        0.0
                    });
                }
                out
            }
        };
        stream.projection = projection;
        Some(stream)
    }

    /// Layout's line: each stamp moved along Direction so it starts Spacing
    /// past where the one before ends, and the whole line is centred on
    /// Position. `unit` is a stamp's size at a point size of 100, across and
    /// down, which is [`fit`](Self::fit)'s for the picture it shows. Stamps of
    /// one point (Every layer) share a place, as wide as the widest of them.
    /// Offset x / y still move each stamp from its place.
    pub fn laid_out(self, stamps: &mut PointsStream, unit: impl Fn(usize) -> [f32; 2]) {
        let axis = usize::from(self.dist_direction == 1);
        let centre = if axis == 0 {
            self.dist_position_x
        } else {
            self.dist_position_y
        };
        // How long each point's place is along the line, by its id.
        let mut places: Vec<(u64, f32)> = Vec::new();
        let mut extents = Vec::with_capacity(stamps.len());
        for i in 0..stamps.len() {
            let size = stamps.size.get(i).copied().unwrap_or(0.0);
            let long = (unit(i)[axis] * size * stamps.stretch_of(i)[axis]).max(0.0);
            extents.push(long);
            let id = stamps.id.get(i).copied().unwrap_or(0);
            match places.iter_mut().find(|(at, _)| *at == id) {
                Some((_, l)) => *l = l.max(long),
                None => places.push((id, long)),
            }
        }
        places.sort_by_key(|(id, _)| *id);
        let total: f32 = places.iter().map(|(_, l)| l).sum::<f32>()
            + self.dist_spacing * places.len().saturating_sub(1) as f32;
        let mut start = centre - total * 0.5;
        let mut from = Vec::with_capacity(places.len());
        for (id, long) in &places {
            from.push((*id, start, *long));
            start += long + self.dist_spacing;
        }
        let anchor = if axis == 0 {
            self.anchor_x
        } else {
            self.anchor_y
        } / 100.0;
        for (i, p) in stamps.position.iter_mut().enumerate() {
            let id = stamps.id.get(i).copied().unwrap_or(0);
            let Some((_, left, long)) = from.iter().find(|(at, ..)| *at == id) else {
                continue;
            };
            // Centred in its place, with its anchor on the point.
            let own = extents.get(i).copied().unwrap_or(0.0);
            p[axis] += left + (long - own) * 0.5 + anchor * own - centre;
        }
    }

    /// The clone layer rows that name a layer, in row order, each with the
    /// layer it names.
    #[must_use]
    pub fn rows(e: &EffectInstance) -> Vec<(&'static str, Uuid)> {
        let schema: &'static EffectSchema = &<Self as EffectMetadata>::SCHEMA;
        schema
            .layer_inputs()
            .filter(|row| *row != PATH_LAYER)
            .filter_map(|row| Some((row, e.layer_ref(row)?)))
            .collect()
    }

    /// Whether the copies are rendered one per clone: Per clone is on Auto
    /// and a Clone index driver is somewhere a render of a layer being cloned
    /// reads it, inside a precomp or a node graph included
    /// ([`clone_index::reaches`]).
    ///
    /// Point age needs every stamp free to take any moment, so it wins and
    /// the copies are rendered alike.
    #[must_use]
    pub fn per_clone_in(self, e: &EffectInstance, doc: &Document, comp: &Composition) -> bool {
        self.per_clone == 0
            && self.time_offset != TIME_POINT_AGE
            && Self::rows(e).iter().any(|(_, id)| {
                comp.layers
                    .iter()
                    .any(|l| l.id == *id && clone_index::reaches(doc, l))
            })
    }

    /// Every picture this effect asks the render for at composition time `t`,
    /// in list order. `None` is a hole: a row naming a layer that is gone.
    ///
    /// The planner and the builder both ask here, so the footage one fetches
    /// is the footage the other draws. `lt` is the time of the layer the
    /// effect is on. `points` is as [`pictures`](Self::pictures) takes it,
    /// and the planner, which has no stream, passes `None` and so fetches for
    /// every render there could be.
    #[must_use]
    pub fn planned(
        e: &EffectInstance,
        doc: &Document,
        comp: &Composition,
        t: f64,
        lt: f64,
        points: Option<usize>,
    ) -> Vec<Option<Planned>> {
        let stored = Self::stored(e, lt);
        let rows = Self::rows(e);
        let of = stored.pictures(rows.len(), stored.per_clone_in(e, doc, comp), points);
        (0..of.len())
            .map(|k| {
                let picture = stored.picture(of, k);
                let (row, layer) = *rows.get(picture.layer)?;
                let start = comp.layers.iter().find(|l| l.id == layer)?.in_point;
                Some(Planned {
                    row,
                    layer,
                    time: stored.moment_time(picture.moment, t, start.0.to_f64()),
                    clone: picture.clone,
                })
            })
            .collect()
    }

    /// How the list the render handed over is laid out, read back from the
    /// resolved bag and the list's own length. What the builder decided is
    /// what the draw reads, so a stamp's picture is the one made for it.
    #[must_use]
    pub fn handed(p: Params<'_>, handed: usize) -> Pictures {
        // A bag nobody resolved has no layer count, and the list is then one
        // picture a layer.
        let layers = usize::try_from(p.int(Self::DERIVED_LAYERS, handed as i32)).unwrap_or(0);
        if layers == 0 || handed == 0 {
            return Pictures::plain(0);
        }
        if p.bool(Self::DERIVED_PER_CLONE, false) {
            Pictures {
                layers,
                moments: 1,
                renders: handed,
            }
        } else {
            Pictures {
                layers,
                moments: (handed / layers).max(1),
                renders: 0,
            }
        }
    }

    /// How a stamp of a picture `picture` px@comp across and down sits on its
    /// point: the Size, Anchor and Corner radius rows as the draw reads them.
    ///
    /// `px_scale` is the raster factor the bag's px rows were scaled by. A
    /// stamp's shape is worked out at composition size, since a point's size
    /// carries the raster factor already.
    #[must_use]
    pub fn fit(self, picture: [f32; 2], px_scale: f32) -> SpriteFit {
        let k = px_scale.max(1e-6);
        let layer = [picture[0].max(1.0), picture[1].max(1.0)];
        let cell = [
            (self.cell_width / k).max(1e-3),
            (self.cell_height / k).max(1e-3),
        ];
        // How many times the layer's width and height the cell is.
        let (across, down) = (cell[0] / layer[0], cell[1] / layer[1]);
        // The stamp's own size at a point size of 100, and how much of it the
        // picture spans.
        let (own, uv) = match self.fit {
            1 => (layer, [1.0; 2]),
            // One scale for both axes: the smaller fits the layer inside the
            // cell, the larger covers it.
            2 => (cell, [across / across.min(down), down / across.min(down)]),
            3 => (cell, [across / across.max(down), down / across.max(down)]),
            _ => ([100.0; 2], [1.0; 2]),
        };
        SpriteFit {
            unit: [own[0] / 100.0, own[1] / 100.0],
            anchor: [self.anchor_x / 100.0, self.anchor_y / 100.0],
            uv,
            corner: (self.corner_radius / k).max(0.0),
        }
    }

    /// How the stamps are drawn — Sprite mode, and the host Mix.
    #[must_use]
    pub fn draw_style(self) -> points::DrawStyle {
        points::DrawStyle {
            mode: points::RenderMode::Sprite,
            feather: 0.0,
            streak_seconds: 0.0,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}
