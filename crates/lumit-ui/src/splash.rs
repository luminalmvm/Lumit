//! Boot splash (decision K-008, docs/15-DESIGN.md §brand).
//!
//! In plain terms: the app starts as a small centred card that lists each
//! module as it comes up (the boot log), then the same window expands into
//! the application. The log is real plumbing — future effect and plugin
//! registries append to it, so slow loads are visible and attributable.

use crate::theme::Theme;
use std::time::Instant;

const LINE_MS: u64 = 140;
const MIN_DWELL_MS: u64 = 1100;

pub struct BootLine {
    pub text: String,
    pub failed: bool,
}

impl BootLine {
    pub fn ok(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            failed: false,
        }
    }
}

pub struct Splash {
    started: Instant,
    pub lines: Vec<BootLine>,
}

impl Splash {
    pub fn new(lines: Vec<BootLine>) -> Self {
        Self {
            started: Instant::now(),
            lines,
        }
    }

    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    pub fn revealed(&self) -> usize {
        let by_time = (self.elapsed_ms() / LINE_MS) as usize + 1;
        by_time.min(self.lines.len())
    }

    pub fn progress(&self) -> f32 {
        let total = MIN_DWELL_MS.max(LINE_MS * self.lines.len() as u64);
        (self.elapsed_ms() as f32 / total as f32).min(1.0)
    }

    pub fn finished(&self) -> bool {
        self.progress() >= 1.0
    }
}

/// Paint the Lumit mark from theme tokens (docs/15-DESIGN.md §brand: the
/// mark is pure strokes — never a raster asset). `rect` is the bounding box.
pub fn paint_mark(painter: &egui::Painter, rect: egui::Rect, theme: &Theme) {
    let c = rect.center();
    let r = rect.width().min(rect.height()) * 0.5;
    let s = r / 84.0; // stroke scale relative to the 256-space construction

    let hex = |radius: f32| -> Vec<egui::Pos2> {
        [90.0_f32, 150.0, 210.0, 270.0, 330.0, 30.0]
            .iter()
            .map(|deg| {
                let a = deg.to_radians();
                egui::pos2(c.x + radius * a.cos(), c.y - radius * a.sin())
            })
            .collect()
    };

    let outer = hex(r);
    let inner = hex(r * 0.5);

    // facet spokes
    for p in &outer {
        painter.line_segment([c, *p], egui::Stroke::new(2.5 * s, theme.hairline_strong));
    }
    // inner facet ring
    for i in 0..6 {
        painter.line_segment(
            [inner[i], inner[(i + 1) % 6]],
            egui::Stroke::new(2.5 * s, theme.hairline_strong),
        );
    }
    // outline
    for i in 0..6 {
        painter.line_segment(
            [outer[i], outer[(i + 1) % 6]],
            egui::Stroke::new(5.0 * s, theme.text_disabled),
        );
    }
    // the K, in clay (ratios from the SVG construction)
    let k = egui::Stroke::new(7.0 * s, theme.accent);
    let stem_x = c.x - r / 3.0;
    painter.line_segment(
        [
            egui::pos2(stem_x, c.y - 0.595 * r),
            egui::pos2(stem_x, c.y + 0.595 * r),
        ],
        k,
    );
    let mid = egui::pos2(stem_x, c.y);
    painter.line_segment([mid, egui::pos2(c.x + 0.357 * r, c.y - 0.571 * r)], k);
    painter.line_segment([mid, egui::pos2(c.x + 0.357 * r, c.y + 0.571 * r)], k);
}

/// The splash card's own size. The card is drawn centred at exactly this
/// size whatever the window measures, so the boot screen looks the same
/// whether the window *is* the card (Windows, macOS — a small frameless
/// window that grows into the app) or merely contains it (Linux, where the
/// window opens at working size because Wayland does not let a client resize
/// itself; see `lumit-app`'s `main`).
pub const CARD: egui::Vec2 = egui::vec2(460.0, 300.0);

/// Render the splash card; returns true when boot display has finished.
pub fn show(ctx: &egui::Context, theme: &Theme, splash: &Splash) -> bool {
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(theme.surface_0))
        .show(ctx, |ui| {
            // Centred card, not the whole window (K-008 asks for a *small
            // centred* splash): on a card-sized window this is the window.
            let full = egui::Rect::from_center_size(ui.max_rect().center(), CARD);

            // the mark
            let mark = egui::Rect::from_center_size(
                egui::pos2(full.center().x, full.top() + 74.0),
                egui::vec2(96.0, 96.0),
            );
            paint_mark(ui.painter(), mark, theme);

            // wordmark + version
            ui.painter().text(
                egui::pos2(full.center().x, full.top() + 138.0),
                egui::Align2::CENTER_CENTER,
                "Lumit",
                egui::FontId::proportional(18.0),
                theme.text_primary,
            );
            ui.painter().text(
                egui::pos2(full.center().x, full.top() + 156.0),
                egui::Align2::CENTER_CENTER,
                env!("CARGO_PKG_VERSION"),
                egui::FontId::monospace(10.0),
                theme.text_disabled,
            );

            // boot log
            let mut y = full.top() + 178.0;
            for line in splash.lines.iter().take(splash.revealed()) {
                let colour = if line.failed {
                    theme.warning
                } else {
                    theme.text_muted
                };
                ui.painter().text(
                    egui::pos2(full.center().x, y),
                    egui::Align2::CENTER_CENTER,
                    &line.text,
                    egui::FontId::monospace(10.0),
                    colour,
                );
                y += 15.0;
            }

            // clay progress hairline along the bottom
            let w = full.width() * splash.progress();
            ui.painter().line_segment(
                [
                    egui::pos2(full.left(), full.bottom() - 1.0),
                    egui::pos2(full.left() + w, full.bottom() - 1.0),
                ],
                egui::Stroke::new(2.0_f32, theme.accent),
            );
        });

    ctx.request_repaint_after(std::time::Duration::from_millis(40));
    splash.finished()
}
