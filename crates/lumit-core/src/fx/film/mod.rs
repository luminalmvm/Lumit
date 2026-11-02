//! Film: a photographic film stock and its print, worked out from the stock's
//! measured spectra. Runs on the render thread, as every effect's CPU half does.
//!
//! **In plain terms.** Light lands on the negative through each of its three
//! layers' own colour sensitivity. Light that goes through the film bounces
//! back off its base and lands on the red layer again a little way off, which
//! is halation. The layers then develop to dye along the stock's measured
//! curves, holding each other back as they do (the couplers), and the dye forms
//! in grains. The negative is printed onto print stock with a lamp, and the
//! print is shown with the light it was made for.
//!
//! The model, its constants and the stock data are Andrea Volpato's spektrafilm
//! (<https://github.com/andreavolpato/spektrafilm>, GPL 3). Its stock profiles
//! are CC BY-SA 4.0, and `stocks.json` keeps that licence; `bake.py` says what
//! was cut. The parts spektrafilm leaves out (gate weave, flicker) are Lumit's.
//!
//! Every pass both renderers run is planned here, once, in raster pixels, and
//! the tables they read come from [`tables`]. [`cpu`] is the oracle.

pub mod cpu;
pub mod presets;
pub mod tables;

use std::sync::OnceLock;

use serde::Deserialize;

/// The stocks a camera loads, in the order Film's Stock list shows them.
pub const FILM_STOCKS: [&str; 17] = [
    "kodak_vision3_50d",
    "kodak_vision3_250d",
    "kodak_vision3_200t",
    "kodak_vision3_500t",
    "kodak_portra_160",
    "kodak_portra_400",
    "kodak_portra_800",
    "kodak_ektar_100",
    "kodak_gold_200",
    "kodak_ultramax_400",
    "fujifilm_c200",
    "fujifilm_pro_400h",
    "fujifilm_xtra_400",
    "kodak_ektachrome_100",
    "kodak_kodachrome_64",
    "fujifilm_provia_100f",
    "fujifilm_velvia_100",
];

/// The stocks a negative is printed on, in the order the Print stock list
/// shows them.
pub const PRINT_STOCKS: [&str; 4] = [
    "kodak_2383",
    "kodak_2393",
    "kodak_portra_endura",
    "fujifilm_crystal_archive_typeii",
];

/// The frame's long edge on the film, millimetres, for each Format.
pub const FORMATS_MM: [f32; 6] = [5.79, 10.26, 12.52, 24.89, 36.0, 52.48];

/// The spectra are sampled from 380 to 780 nm, 5 nm apart.
pub const WAVES: usize = 81;

/// One stock as `stocks.json` carries it.
#[derive(Debug, Deserialize)]
pub struct Stock {
    pub id: String,
    /// "negative", "reversal" or "print".
    pub kind: String,
    /// "cine" or "still".
    #[serde(rename = "use")]
    pub usage: String,
    /// How well the anti-halation backing works: "strong", "weak" or "no".
    pub backing: String,
    /// Each dye's density, per wavelength.
    pub dye: Vec<[f32; 3]>,
    /// The base and fog's density, per wavelength.
    pub base: Vec<f32>,
    /// The light it is looked at in.
    pub view: String,
    /// Density by log exposure, 256 samples from -3 to 4.
    pub curves: Vec<[f32; 3]>,
    /// A print stock's layers' sensitivity, per wavelength.
    #[serde(default)]
    pub sens: Vec<[f32; 3]>,
    /// The couplers: donor layer by row, the layer held back by column.
    #[serde(default)]
    pub couplers: [[f32; 3]; 3],
    /// Grain variance by density, density² µm², 64 samples up to each
    /// layer's top.
    #[serde(default)]
    pub grain: Vec<[f32; 3]>,
    /// Linear Rec. 709 to the layers' exposure, grey at one.
    #[serde(default)]
    pub camera: [[f32; 3]; 3],
}

/// A light to look at a print or a slide in.
#[derive(Debug, Deserialize)]
pub struct View {
    pub light: Vec<f32>,
    /// XYZ seen in it to linear Rec. 709, adapted to D65.
    pub to_rec709: [[f32; 3]; 3],
}

/// Everything `stocks.json` holds.
#[derive(Debug, Deserialize)]
pub struct Data {
    pub cmf: Vec<[f32; 3]>,
    /// The printer's lamp.
    pub enlarger: Vec<f32>,
    pub views: std::collections::BTreeMap<String, View>,
    pub stocks: Vec<Stock>,
}

impl Data {
    pub fn stock(&self, id: &str) -> Option<&Stock> {
        self.stocks.iter().find(|s| s.id == id)
    }

    /// Every array the right length, so nothing past here indexes off the end.
    fn whole(&self) -> bool {
        let spectral = |s: &Stock| {
            s.dye.len() == WAVES
                && s.base.len() == WAVES
                && s.curves.len() == tables::CURVE
                && (s.kind != "print" || s.sens.len() == WAVES)
                && (s.kind == "print" || s.grain.len() == tables::GRAIN)
        };
        self.cmf.len() == WAVES
            && self.enlarger.len() == WAVES
            && self.views.values().all(|v| v.light.len() == WAVES)
            && self.stocks.iter().all(spectral)
            && FILM_STOCKS.iter().all(|id| self.stock(id).is_some())
            && PRINT_STOCKS.iter().all(|id| self.stock(id).is_some())
    }
}

/// The stock data, parsed once. `None` only if the shipped file were broken,
/// which a test rules out; Film then leaves the picture as it is.
pub fn data() -> Option<&'static Data> {
    static DATA: OnceLock<Option<Data>> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str::<Data>(include_str!("stocks.json"))
            .ok()
            .filter(Data::whole)
    })
    .as_ref()
}

/// How far light spreads in one blur of a pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Spread {
    /// A gaussian, σ in raster pixels.
    Gauss(f32),
    /// Glow's sum of five round exponentials, each twice as wide as the last.
    Exp { radius_px: f32, falloff: f32 },
}

/// One spreading pass: the picture kept, plus each blur of it, per channel.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Stage {
    pub keep: [f32; 3],
    pub terms: Vec<(Spread, [f32; 3])>,
}

impl Stage {
    /// A gaussian narrower than this is the picture itself to within a few
    /// thousandths, so its weight goes to `keep` and no pass runs.
    const NARROWEST: f32 = 0.3;

    pub fn gauss(&mut self, sigma_px: f32, weight: [f32; 3]) {
        if sigma_px < Self::NARROWEST {
            for (keep, w) in self.keep.iter_mut().zip(weight) {
                *keep += w;
            }
        } else {
            self.terms.push((Spread::Gauss(sigma_px), weight));
        }
    }

    /// A round exponential of decay length `lambda_px`, as the gaussian that
    /// spreads light as far on average: σ = √3 λ. spektrafilm sums three
    /// gaussians for it; on a tail this faint one does the same job.
    pub fn exponential(&mut self, lambda_px: f32, weight: [f32; 3]) {
        self.gauss(3f32.sqrt() * lambda_px, weight);
    }

    /// Whether the pass changes anything at all.
    pub fn is_identity(&self) -> bool {
        self.terms.is_empty() && self.keep == [1.0; 3]
    }
}

/// The halation of one stock: how wide the first bounce is, µm, and how much
/// of each layer's light comes back.
pub fn halation_preset(usage: &str, backing: &str) -> (f32, [f32; 3]) {
    // spektrafilm's runtime/params_builder.py: cine bases are thinner.
    let sigma = if usage == "cine" { 50.0 } else { 65.0 };
    let strength = match backing {
        "strong" => [0.015, 0.005, 0.0],
        "weak" => [0.08, 0.02, 0.0],
        _ => [0.30, 0.10, 0.015],
    };
    (sigma, strength)
}

/// The negative's gamma for a push or pull in stops, from the ECN-2
/// development times: 150 s for a pull, 220 and 280 s for one and two pushes,
/// against 180 s.
pub fn push_gamma(stops: f32) -> f32 {
    let points = [
        (-1.0, 150.0 / 180.0),
        (0.0, 1.0),
        (1.0, 220.0 / 180.0),
        (2.0, 280.0 / 180.0),
    ];
    let s = stops.clamp(-1.0, 2.0);
    let mut out = 1.0;
    for pair in points.windows(2) {
        let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
        if s >= x0 && s <= x1 {
            out = y0 + (y1 - y0) * (s - x0) / (x1 - x0);
        }
    }
    out
}

/// Gate weave at layer time `t`: where the frame sits, as a share of its
/// width across and down. A slow wander with a little jitter per frame,
/// `amount` 1 being about a tenth of a per cent of the width either way.
pub fn weave(seed: u32, t: f64, amount: f32, speed: f32) -> [f32; 2] {
    let at = (t * f64::from(speed.max(0.0))) as f32;
    let step = (t * 1000.0).round() as i32;
    let lane = |l: u32| {
        let slow = super::noise::value3(seed, l, at, 0.0, 0.0, 0);
        let jitter = super::noise::hash01(seed, l + 2, step, 0, 0) * 2.0 - 1.0;
        (slow * 0.8 + jitter * 0.2) * 0.001 * amount
    };
    [lane(0), lane(1) * 0.6]
}

/// Flicker at layer time `t`, stops: a fresh exposure each frame, `amount` 1
/// being about a twentieth of a stop either way.
pub fn flicker(seed: u32, t: f64, amount: f32) -> f32 {
    let step = (t * 1000.0).round() as i32;
    let jitter = super::noise::hash01(seed, 7, step, 0, 0) * 2.0 - 1.0;
    let slow = super::noise::value3(seed, 8, t as f32 * 0.7, 0.0, 0.0, 0);
    (jitter * 0.6 + slow * 0.4) * 0.05 * amount
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use crate::fx::effects::film::Film;
    use crate::fx::params::Params;
    use crate::fx::registry::EffectMetadata;

    fn flat(v: f32, n: usize) -> Vec<f32> {
        (0..n * n).flat_map(|_| [v, v, v, 1.0]).collect()
    }

    /// **Every stock prints a grey card as a grey card**, at about the grey it
    /// was: the printer lights are timed from the stock's spectra, so a
    /// broken table or timing shows up as a cast here.
    #[test]
    fn every_stock_keeps_grey_grey() {
        for stock in 0..super::FILM_STOCKS.len() as u32 {
            for print_stock in 0..super::PRINT_STOCKS.len() as u32 {
                let film = Film {
                    stock,
                    print_stock,
                    grain: false,
                    halation: false,
                    ..Film::read(Params::EMPTY)
                };
                let frame = film.frame(Params::EMPTY, 8, 8).expect("the stocks load");
                let mut img = flat(0.184, 8);
                super::cpu::film(&mut img, 8, 8, &frame);
                let [r, g, b] = [img[0], img[1], img[2]];
                let (low, high) = (r.min(g).min(b), r.max(g).max(b));
                // A slide is shown as it is, its own balance and all, so it
                // only has to be close. A print is timed to grey.
                let slide = frame.k.shown == crate::fx::effects::film::Shown::Slide;
                let most = if slide { 1.1 } else { 1.04 };
                assert!(
                    high / low < most,
                    "stock {stock} on {print_stock}: {r} {g} {b}"
                );
                assert!(
                    (0.14..0.23).contains(&g),
                    "stock {stock} on {print_stock}: grey at {g}"
                );
            }
        }
    }

    /// Every part at once, on a ramp with lights in it: nothing comes out
    /// negative or not a number.
    #[test]
    fn every_part_at_once_stays_finite() {
        let (w, h) = (24u32, 16u32);
        let mut img: Vec<f32> = (0..w * h)
            .flat_map(|i| {
                let v = (i % w) as f32 / w as f32 * 2.0 + if i % 37 == 0 { 30.0 } else { 0.0 };
                [v, v * 0.7, v * 0.4, 1.0]
            })
            .collect();
        for film in [
            Film {
                bloom: true,
                weave: true,
                flicker: true,
                highlights: 2.0,
                ..Film::read(Params::EMPTY)
            },
            Film {
                print: false,
                backing: 3,
                ..Film::read(Params::EMPTY)
            },
            Film {
                colour: false,
                format: 0,
                ..Film::read(Params::EMPTY)
            },
        ] {
            let frame = film.frame(Params::EMPTY, w, h).expect("the stocks load");
            super::cpu::film(&mut img, w, h, &frame);
            assert!(img.iter().all(|v| v.is_finite() && *v >= 0.0));
        }
        // The shipped looks are Films too, Halation among them.
        let looks = super::presets::presets();
        assert!(looks.iter().any(|(name, _)| *name == "Halation"));
        assert!(looks
            .iter()
            .all(|(_, inst)| inst.effect.match_name == "film"));
    }
}
