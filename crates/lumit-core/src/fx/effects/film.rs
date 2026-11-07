//! Film: the picture shot on a film stock and printed, with halation, bloom,
//! grain, gate weave and flicker each its own part to turn on or off. The
//! model is in [`crate::fx::film`].

use crate::fx::{
    film, film::tables, film::Spread, film::Stage, params::ParamId, params::Params, params::Value,
    registry::EffectDef, registry::EffectMetadata, registry::ResolveCx, schema::EffectSchema,
    schema::EnabledCond, schema::EnabledWhen, schema::ParamGroup,
};
use lumit_fx_macros::Effect;

pub const FILM_GROUPS: &[ParamGroup] = &[
    group("Negative", &["colour", "push", "interlayer"]),
    group(
        "Print",
        &[
            "print",
            "print_stock",
            "print_brightness",
            "contrast",
            "red",
            "green",
            "blue",
        ],
    ),
    // Open: halation is what most people come to Film for.
    ParamGroup {
        collapsed: false,
        ..group(
            "Halation",
            &["halation", "halation_amount", "halation_size", "backing"],
        )
    },
    group("Bloom", &["bloom", "bloom_amount", "bloom_size"]),
    group(
        "Grain",
        &[
            "grain",
            "grain_amount",
            "grain_size",
            "grain_colour",
            "animate",
            "seed",
        ],
    ),
    group("Gate weave", &["weave", "weave_amount", "weave_speed"]),
    group("Flicker", &["flicker", "flicker_amount"]),
];

const fn group(label: &'static str, params: &'static [&'static str]) -> ParamGroup {
    ParamGroup {
        label,
        params,
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    }
}

const fn on(param: &'static str, part: &'static str) -> EnabledWhen {
    EnabledWhen {
        param,
        on: part,
        cond: EnabledCond::BoolIs(true),
    }
}

pub const FILM_ENABLED_WHEN: &[EnabledWhen] = &[
    on("push", "colour"),
    on("interlayer", "colour"),
    on("print", "colour"),
    on("print_stock", "print"),
    on("print_brightness", "print"),
    on("contrast", "print"),
    on("red", "print"),
    on("green", "print"),
    on("blue", "print"),
    on("halation_amount", "halation"),
    on("halation_size", "halation"),
    on("backing", "halation"),
    on("bloom_amount", "bloom"),
    on("bloom_size", "bloom"),
    on("grain_amount", "grain"),
    on("grain_size", "grain"),
    on("grain_colour", "grain"),
    on("animate", "grain"),
    on("seed", "grain"),
    on("weave_amount", "weave"),
    on("weave_speed", "weave"),
    on("flicker_amount", "flicker"),
];

/// Film's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "film",
    label = "Film",
    version = 1,
    category = Stylise,
    cost = Heavy,
    // Halation and bloom carry light across the frame.
    roi = FullFrame,
    // The stock's curves read the colour, not the colour times its coverage.
    premultiplied = false,
    seeded = true,
    groups = FILM_GROUPS,
    enabled_when = FILM_ENABLED_WHEN,
)]
pub struct Film {
    /// The stock in the camera: motion picture negatives, then stills
    /// negatives, then slides.
    #[choice(
        options = [
            "Vision3 50D",
            "Vision3 250D",
            "Vision3 200T",
            "Vision3 500T",
            "Portra 160",
            "Portra 400",
            "Portra 800",
            "Ektar 100",
            "Gold 200",
            "Ultramax 400",
            "Fujicolor C200",
            "Fujicolor Pro 400H",
            "Superia X-TRA 400",
            "Ektachrome 100",
            "Kodachrome 64",
            "Provia 100F",
            "Velvia 100",
        ],
        default = 3,
        dividers_after = &[3, 12]
    )]
    pub stock: u32,

    /// How big the frame is on the film. A smaller frame is enlarged more, so
    /// its grain and halation look bigger.
    #[choice(
        options = ["Super 8", "16 mm", "Super 16", "35 mm", "35 mm stills", "65 mm"],
        default = 3
    )]
    pub format: u32,

    /// Stops of light on the negative.
    #[slider(min = -4.0, max = 4.0, default = 0.0, hard_min = -10.0, hard_max = 10.0, unit = Raw)]
    pub exposure: f32,

    /// Stops the picture's brightest parts are lifted by, from just below
    /// white, so light a camera clipped reaches the film as bright as it was.
    #[slider(label = "Highlights", min = 0.0, max = 6.0, default = 0.0, hard_min = 0.0, hard_max = 12.0, unit = Raw)]
    pub highlights: f32,

    // ---- Negative ----
    /// Off keeps the picture's own colour and tone, and adds only the parts
    /// below.
    #[toggle(label = "Film colour", default = true)]
    pub colour: bool,

    /// Stops the negative is pushed (more contrast) or pulled (less) in
    /// development.
    #[slider(label = "Push or pull", min = -1.0, max = 2.0, default = 0.0, hard_min = -1.0, hard_max = 2.0, unit = Raw)]
    pub push: f32,

    /// How hard each layer holds back its neighbours and itself as it
    /// develops: colour separation and edge contrast.
    #[slider(min = 0.0, max = 200.0, default = 100.0, hard_min = 0.0, hard_max = 400.0, unit = Percent)]
    pub interlayer: f32,

    // ---- Print ----
    /// Off shows a scan of the negative instead of a print of it. Slides are
    /// never printed.
    #[toggle(label = "On", default = true)]
    pub print: bool,

    #[choice(
        label = "Print stock",
        options = ["Vision 2383", "Vision Premier 2393", "Portra Endura paper", "Crystal Archive paper"],
        default = 0,
        dividers_after = &[1]
    )]
    pub print_stock: u32,

    /// Stops brighter or darker the print is made, by printing it with less
    /// light or more.
    #[slider(label = "Brightness", min = -2.0, max = 2.0, default = 0.0, hard_min = -6.0, hard_max = 6.0, unit = Raw)]
    pub print_brightness: f32,

    /// The print's contrast, per cent of the stock's own.
    #[slider(min = 50.0, max = 200.0, default = 100.0, hard_min = 10.0, hard_max = 400.0, unit = Percent)]
    pub contrast: f32,

    /// Printer points: each a twelfth of a stop less of that light on the
    /// print, which leaves more of that colour on the screen.
    #[counter(min = -12, max = 12, default = 0, hard_min = -48, hard_max = 48, unit = Raw)]
    pub red: i32,

    #[counter(min = -12, max = 12, default = 0, hard_min = -48, hard_max = 48, unit = Raw)]
    pub green: i32,

    #[counter(min = -12, max = 12, default = 0, hard_min = -48, hard_max = 48, unit = Raw)]
    pub blue: i32,

    // ---- Halation ----
    /// Light that goes through the film, bounces off its base and comes back
    /// to the red layer: a red and orange glow round bright lights.
    #[toggle(label = "On", default = true)]
    pub halation: bool,

    #[slider(label = "Amount", min = 0.0, max = 400.0, default = 100.0, hard_min = 0.0, hard_max = 4000.0, unit = Percent)]
    pub halation_amount: f32,

    #[slider(label = "Size", min = 25.0, max = 400.0, default = 100.0, hard_min = 5.0, hard_max = 2000.0, unit = Percent)]
    pub halation_size: f32,

    /// The layer behind the film that stops the bounce. Removed is a stock with
    /// its backing taken off, which glows the most.
    #[choice(options = ["The stock's own", "Strong", "Weak", "Removed"], default = 0, dividers_after = &[0])]
    pub backing: u32,

    // ---- Bloom ----
    /// A diffusion filter on the lens: highlights spread a soft glow and the
    /// shadows lift a little.
    #[toggle(label = "On", default = false)]
    pub bloom: bool,

    #[slider(label = "Amount", min = 0.0, max = 100.0, default = 25.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub bloom_amount: f32,

    #[slider(label = "Size", min = 25.0, max = 400.0, default = 100.0, hard_min = 5.0, hard_max = 2000.0, unit = Percent)]
    pub bloom_size: f32,

    // ---- Grain ----
    /// The dye clouds the picture is made of, strongest where the stock's grain
    /// is.
    #[toggle(label = "On", default = true)]
    pub grain: bool,

    #[slider(label = "Amount", min = 0.0, max = 400.0, default = 100.0, hard_min = 0.0, hard_max = 2000.0, unit = Percent)]
    pub grain_amount: f32,

    #[slider(label = "Size", min = 25.0, max = 400.0, default = 100.0, hard_min = 5.0, hard_max = 2000.0, unit = Percent)]
    pub grain_size: f32,

    /// 100 gives each layer its own grain, as film has. 0 makes it one grain
    /// for all three.
    #[slider(label = "Colour", min = 0.0, max = 100.0, default = 100.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub grain_colour: f32,

    /// On, the grain changes every frame.
    #[toggle(default = true)]
    pub animate: bool,

    #[seed]
    pub seed: u32,

    // ---- Gate weave ----
    /// The frame wandering a little in the gate.
    #[toggle(label = "On", default = false)]
    pub weave: bool,

    #[slider(label = "Amount", min = 0.0, max = 400.0, default = 100.0, hard_min = 0.0, hard_max = 2000.0, unit = Percent)]
    pub weave_amount: f32,

    /// How quickly it wanders, in wanders a second.
    #[slider(label = "Speed", min = 0.1, max = 8.0, default = 1.5, hard_min = 0.0, hard_max = 50.0, unit = Raw)]
    pub weave_speed: f32,

    // ---- Flicker ----
    /// The exposure changing a little from frame to frame.
    #[toggle(label = "On", default = false)]
    pub flicker: bool,

    #[slider(label = "Amount", min = 0.0, max = 400.0, default = 100.0, hard_min = 0.0, hard_max = 2000.0, unit = Percent)]
    pub flicker_amount: f32,

    #[slider(min = 0.0, max = 100.0, default = 100.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub mix: f32,
}

/// What Film draws a frame with: the numbers every pass reads, the passes that
/// spread light, and the tables. Both renderers take it as it is.
#[derive(Debug, Clone, PartialEq)]
pub struct FilmFrame {
    pub k: FilmKernel,
    pub bloom: Stage,
    pub scatter: Stage,
    pub halation: Stage,
    /// The couplers' reach. Empty when they are off.
    pub couplers: Stage,
    /// The grain's own softness, σ in raster pixels.
    pub grain_blur: f32,
    pub tables: Vec<f32>,
}

/// How the developed film reaches the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    Print = 0,
    /// The negative scanned and inverted.
    Scan = 1,
    /// A slide, as it is.
    Slide = 2,
    /// Film colour off: the picture's own.
    Own = 3,
}

/// The per-pixel numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilmKernel {
    /// Linear Rec. 709 to the layers' exposure, with the exposure in it.
    pub camera: [[f32; 3]; 3],
    /// Stops the clipped highlights are lifted by.
    pub boost: f32,
    /// The layers' exposure back to linear Rec. 709, for a scan.
    pub inverse: [[f32; 3]; 3],
    /// The couplers, donor by row, scaled by Interlayer.
    pub couplers: [[f32; 3]; 3],
    pub couplers_on: bool,
    pub shown: Shown,
    pub positive: bool,
    pub gamma: f32,
    /// Each layer's top density, its density at grey and its slope there.
    pub top: [f32; 3],
    pub grey: [f32; 3],
    pub slope: [f32; 3],
    /// What the grain table is scaled by: the amount over the pixel's size.
    /// Zero is no grain.
    pub grain: f32,
    pub grain_colour: f32,
    pub seed: u32,
    pub tick: i32,
    /// log10 of the printer lights.
    pub print_gain: [f32; 3],
    pub print_gamma: f32,
    pub scan_low: [f32; 3],
    pub scan_top: [f32; 3],
    pub to_rec709: [[f32; 3]; 3],
    /// Where the frame sits, raster pixels.
    pub weave: [f32; 2],
    pub mix: f32,
}

impl Film {
    pub const DERIVED_TICK: ParamId = ParamId::new("derived.tick");

    /// The frame for a `w × h` raster, or `None` to leave the picture alone.
    pub fn frame(&self, p: Params<'_>, w: u32, h: u32) -> Option<FilmFrame> {
        let film_ix = (self.stock as usize).min(film::FILM_STOCKS.len() - 1);
        let data = film::data()?;
        let stock = data.stock(film::FILM_STOCKS[film_ix])?;
        let positive = stock.kind == "reversal";
        let colour = self.colour;
        let printed = colour && self.print && !positive;
        let print_ix = (self.print_stock as usize).min(film::PRINT_STOCKS.len() - 1);
        let pair = tables::pair(film_ix, printed.then_some(print_ix))?;
        let (top, grey, slope) = tables::grey_and_slope(film_ix)?;

        let format = film::FORMATS_MM[(self.format as usize).min(film::FORMATS_MM.len() - 1)];
        let pixel_um = format * 1000.0 / w.max(h).max(1) as f32;
        let tick = p.int(Self::DERIVED_TICK, 0);
        let t = f64::from(tick) / 1000.0;
        let flicker = if self.flicker {
            film::flicker(self.seed, t, self.flicker_amount.max(0.0) / 100.0)
        } else {
            0.0
        };
        let weave = if self.weave {
            film::weave(
                self.seed,
                t,
                self.weave_amount.max(0.0) / 100.0,
                self.weave_speed,
            )
        } else {
            [0.0, 0.0]
        };
        let gain = (self.exposure + flicker).exp2();
        let camera = if colour {
            stock.camera
        } else {
            let k = 1.0 / 0.184;
            [[k, 0.0, 0.0], [0.0, k, 0.0], [0.0, 0.0, k]]
        };
        let interlayer = self.interlayer.max(0.0) / 100.0;
        let couplers_on = colour && interlayer > 0.0;
        let couplers = stock.couplers.map(|r| r.map(|v| v * interlayer));
        let points = [self.red, self.green, self.blue].map(|p| p as f32);
        let lights = (0..3).map(|c| {
            pair.neutral[c] - (self.print_brightness + points[c] / 12.0) * std::f32::consts::LOG10_2
        });
        let mut print_gain = [0.0f32; 3];
        for (g, v) in print_gain.iter_mut().zip(lights) {
            *g = v;
        }
        let shown = match (colour, printed, positive) {
            (false, _, _) => Shown::Own,
            (true, _, true) => Shown::Slide,
            (true, true, false) => Shown::Print,
            (true, false, false) => Shown::Scan,
        };

        let k = FilmKernel {
            camera: camera.map(|r| r.map(|v| v * gain)),
            boost: self.highlights.max(0.0),
            inverse: invert(&stock.camera),
            couplers,
            couplers_on,
            shown,
            positive,
            gamma: if colour {
                film::push_gamma(self.push)
            } else {
                1.0
            },
            top,
            grey,
            slope,
            grain: if self.grain {
                self.grain_amount.max(0.0) / 100.0 * (self.grain_size.max(0.0) / 100.0) / pixel_um
            } else {
                0.0
            },
            grain_colour: (self.grain_colour / 100.0).clamp(0.0, 1.0),
            seed: self.seed,
            tick: if self.animate { tick } else { 0 },
            print_gain,
            print_gamma: (self.contrast / 100.0).max(0.01),
            scan_low: pair.scan_low,
            scan_top: pair.scan_top,
            to_rec709: pair.to_rec709,
            weave: weave.map(|v| v * w as f32),
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        };

        let px = |um: f32| um / pixel_um;
        let mut bloom = Stage {
            keep: [1.0; 3],
            terms: Vec::new(),
        };
        if self.bloom {
            // A mist filter: five round exponentials from 30 µm, each twice the
            // last, the light falling by about a half each time, which is the
            // r^-2.9 tail spektrafilm fits to its classic mist.
            let f = (self.bloom_amount / 100.0).clamp(0.0, 1.0) * 0.75;
            let size = self.bloom_size.max(0.0) / 100.0;
            bloom.keep = [1.0 - f; 3];
            let radius_px = px(30.0 * size) / crate::fx::cpu::GLOW_CORE;
            bloom.terms.push((
                Spread::Exp {
                    radius_px,
                    falloff: 1.15,
                },
                [f; 3],
            ));
        }
        let mut scatter = Stage {
            keep: [0.0; 3],
            terms: Vec::new(),
        };
        let mut halation = Stage {
            keep: [1.0; 3],
            terms: Vec::new(),
        };
        if self.halation {
            // In-emulsion scatter, a gaussian core and an exponential tail
            // that keep all the light (spektrafilm's apply_halation_um).
            let core = [2.2, 2.0, 1.6];
            let tail = [9.3, 9.7, 9.1];
            let tail_share = [0.78, 0.65, 0.67];
            let mean = |v: [f32; 3]| (v[0] + v[1] + v[2]) / 3.0;
            scatter.gauss(px(mean(core)), tail_share.map(|t| 1.0 - t));
            scatter.exponential(px(mean(tail)), tail_share);
            // The bounces off the base: three, each wider by √k and half as
            // strong, renormalised so the light adds to one (spektrafilm).
            let backing = match self.backing {
                1 => "strong",
                2 => "weak",
                3 => "no",
                _ => stock.backing.as_str(),
            };
            let (sigma, strength) = film::halation_preset(&stock.usage, backing);
            let amount = self.halation_amount.max(0.0) / 100.0;
            let a = strength.map(|s| s * amount);
            let size = self.halation_size.max(0.0) / 100.0;
            halation.keep = a.map(|a| 1.0 / (1.0 + a));
            // The second and third bounces as one gaussian that spreads their
            // light as far (σ√(7/3)), carrying their three sevenths.
            for (k, share) in [(1.0f32, 4.0 / 7.0), (7.0 / 3.0, 3.0 / 7.0)] {
                halation.gauss(
                    px(sigma * size) * k.sqrt(),
                    a.map(|a| a * share / (1.0 + a)),
                );
            }
        } else {
            scatter.keep = [1.0; 3];
        }
        let mut couplers = Stage::default();
        if couplers_on {
            // Inhibitors spread about 20 µm, with a long thin tail.
            couplers.gauss(px(20.0), [0.94; 3]);
            couplers.exponential(px(200.0), [0.06; 3]);
        }
        let tables = tables::buffer(film_ix, pair, printed.then_some(print_ix), &k.couplers);
        Some(FilmFrame {
            k,
            bloom,
            scatter,
            halation,
            couplers,
            // The dye clouds, about 2 µm across on the film, softened by half a
            // pixel of the scan. The grain's strength is per pixel already
            // (Selwyn), so a wider blur here would only wash it out.
            grain_blur: px(2.0 * self.grain_size.max(0.0) / 100.0).hypot(0.5),
            tables,
        })
    }
}

/// A 3×3's inverse, or the identity for one that has none.
fn invert(m: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if det.abs() < 1e-12 {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let k = 1.0 / det;
    [
        [
            (e * i - f * h) * k,
            (c * h - b * i) * k,
            (b * f - c * e) * k,
        ],
        [
            (f * g - d * i) * k,
            (a * i - c * g) * k,
            (c * d - a * f) * k,
        ],
        [
            (d * h - e * g) * k,
            (b * g - a * h) * k,
            (a * e - b * d) * k,
        ],
    ]
}

/// Film's behaviour.
pub struct FilmDef;

impl EffectDef for FilmDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Film as EffectMetadata>::SCHEMA
    }

    /// The frame, as a whole number of milliseconds of layer time: the grain's
    /// draw, the gate's place and the exposure's flicker all follow from it, so
    /// both renderers and every export agree.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(
            Film::DERIVED_TICK,
            Value::Int((cx.lt * 1000.0).round() as i32),
        );
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        if let Some(frame) = Film::read(p).frame(p, w, h) {
            film::cpu::film(rgba, w, h, &frame);
        }
    }
}
