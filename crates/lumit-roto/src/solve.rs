//! One frame's matte, end to end: the geodesic segmentation, then the refine
//! band (docs/impl/roto.md §2 and §4).

use crate::gdt;
use crate::guided::{box_mean, Guided};
use crate::{check_plane, FrameRgb, RotoError, RotoSettings, Seeds};

/// Solves frames, one at a time, reusing every buffer.
///
/// Allocation happens when the frame size changes and at no other moment: a
/// six-hundred-frame propagation run allocates once (14 §5). Nothing here is
/// shared, so a caller who wants two solves at once holds two solvers.
#[derive(Debug)]
pub struct RotoSolver {
    settings: RotoSettings,
    len: usize,
    width: u32,
    height: u32,
    d_f: Vec<f32>,
    d_b: Vec<f32>,
    alpha_raw: Vec<f32>,
    filtered: Vec<f32>,
    band: Vec<f32>,
    band_dilated: Vec<f32>,
    tmp: Vec<f32>,
    guided: Guided,
}

impl RotoSolver {
    pub fn new(settings: RotoSettings) -> Self {
        Self {
            settings,
            len: 0,
            width: 0,
            height: 0,
            d_f: Vec::new(),
            d_b: Vec::new(),
            alpha_raw: Vec::new(),
            filtered: Vec::new(),
            band: Vec::new(),
            band_dilated: Vec::new(),
            tmp: Vec::new(),
            guided: Guided::default(),
        }
    }

    pub fn settings(&self) -> &RotoSettings {
        &self.settings
    }

    /// The segmentation's own answer for the frame last solved, before the
    /// refine band reshaped it — what the tests compare the filtered edge
    /// against, and what a Boundary view would draw.
    pub fn alpha_raw(&self) -> &[f32] {
        &self.alpha_raw
    }

    /// Solve one frame into `out` (one f32 per pixel, 0..1).
    pub fn solve(
        &mut self,
        frame: FrameRgb<'_>,
        seeds: &Seeds,
        out: &mut [f32],
    ) -> Result<(), RotoError> {
        let (w, h) = (frame.width(), frame.height());
        if seeds.width() != w || seeds.height() != h {
            return Err(RotoError::SizeMismatch {
                a_width: w,
                a_height: h,
                b_width: seeds.width(),
                b_height: seeds.height(),
            });
        }
        let n = check_plane(out.len(), 1, w, h)?;
        let (fg, bg) = seeds.counts();
        if fg == 0 || bg == 0 {
            return Err(RotoError::NoSeeds);
        }
        self.resize(w, h, n);

        gdt::alpha_raw(
            frame,
            seeds,
            self.settings.gamma,
            &mut self.d_f,
            &mut self.d_b,
            &mut self.alpha_raw,
        );

        let radius = self.settings.guide_radius;
        self.guided.filter(
            frame,
            &self.alpha_raw,
            radius,
            self.settings.guide_eps,
            &mut self.filtered,
        );

        // The band: where the segmentation was undecided, widened by the
        // filter's own radius so its window has room to work. Everywhere else
        // the answer is snapped, because a filter run over a solid interior
        // greys it wherever the guide happens to have texture (§12's fence).
        let half = self.settings.band;
        for (i, slot) in self.band.iter_mut().enumerate() {
            let a = self.alpha_raw.get(i).copied().unwrap_or(0.0);
            *slot = if (a - 0.5).abs() < half { 1.0 } else { 0.0 };
        }
        box_mean(
            &self.band,
            w,
            h,
            radius,
            &mut self.tmp,
            &mut self.band_dilated,
        );
        for i in 0..n {
            let raw = self.alpha_raw.get(i).copied().unwrap_or(0.0);
            let in_band = self.band_dilated.get(i).copied().unwrap_or(0.0) > 0.0;
            let refine = seeds.refine_at(i);
            let value = if in_band || refine {
                self.filtered.get(i).copied().unwrap_or(raw).clamp(0.0, 1.0)
            } else if raw > 0.5 {
                1.0
            } else {
                0.0
            };
            if let Some(slot) = out.get_mut(i) {
                *slot = value;
            }
        }
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32, len: usize) {
        if self.len == len && self.width == width && self.height == height {
            return;
        }
        self.len = len;
        self.width = width;
        self.height = height;
        for plane in [
            &mut self.d_f,
            &mut self.d_b,
            &mut self.alpha_raw,
            &mut self.filtered,
            &mut self.band,
            &mut self.band_dilated,
            &mut self.tmp,
        ] {
            plane.clear();
            plane.resize(len, 0.0);
        }
        self.guided.resize(len);
    }
}
