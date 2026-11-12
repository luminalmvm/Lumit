//! Round corners worked out on the outlines: each corner of a contour filleted
//! by the radius.

/// A closed contour with each side shorter than `nick` that cuts a corner off
/// closed back into the corner: the two longer sides either side of it are
/// carried on until they meet. Only where they meet near it, at 60° to 150°,
/// and close enough that the corner's arc reaches back past the side: the end
/// of a thin strip, whose sides run parallel, the tip of a narrow wedge, whose
/// sides meet far off or sharply, a side a cut ran across well short of the
/// corner, and a curve drawn in short pieces are real sides and stay.
///
/// First, the points between two real corners go when the side barely turns at
/// each and all of them sit within a pixel of the straight line from corner to
/// corner: a cut that only grazes a side leaves such points there, and as
/// corners of their own they held the real corners beside them to half their
/// sides, so a shape rounded fully one frame and barely the next.
///
/// "60° or more" allows a hair under, so an equilateral triangle's corners do
/// not fall either side of it from one frame to the next.
#[must_use]
pub fn without_nicks(contour: &[[f32; 2]], nick: f32) -> Vec<[f32; 2]> {
    /// The cosine of a corner of 60°, less a hair (about 59°).
    const WIDE: f32 = 0.515;
    let mut c = contour.to_vec();
    if nick <= 0.0 {
        return c;
    }
    // Each point judged against its own neighbours as they came, all at once,
    // so a long side drawn in many nearly straight pieces cannot be worn away
    // a pixel at a time.
    let n = c.len();
    if n > 3 {
        // Barely turning (a thin ray's corners are as close to the line, and
        // are corners), and within a pixel of it.
        let straight: Vec<bool> = (0..n)
            .map(|i| {
                let (a, p, b) = (c[(i + n - 1) % n], c[i], c[(i + 1) % n]);
                let (e, f) = ([p[0] - a[0], p[1] - a[1]], [b[0] - p[0], b[1] - p[1]]);
                let (le, lf) = (e[0].hypot(e[1]), f[0].hypot(f[1]));
                let d = [b[0] - a[0], b[1] - a[1]];
                let l = d[0].hypot(d[1]);
                le > 1e-6
                    && lf > 1e-6
                    && (e[0] * f[0] + e[1] * f[1]) / (le * lf) > 0.985
                    && (d[0] * e[1] - d[1] * e[0]).abs() / l < 1.0
            })
            .collect();
        // A run of them between two real corners goes when every one of them
        // sits within a pixel of the straight line from corner to corner: a
        // curve's points all barely turn too, but stand off that line, and stay.
        // And only while the corners at its ends stay 30° or more: the points
        // a thin ray's tip is drawn with, closed into one sharp point, would
        // round it off far down the ray.
        let open = |q: [f32; 2], u: [f32; 2], v: [f32; 2]| {
            let (u, v) = ([u[0] - q[0], u[1] - q[1]], [v[0] - q[0], v[1] - q[1]]);
            let (lu, lv) = (u[0].hypot(u[1]), v[0].hypot(v[1]));
            lu > 1e-6 && lv > 1e-6 && (u[0] * v[0] + u[1] * v[1]) / (lu * lv) <= 0.866
        };
        let mut graze = vec![false; n];
        let corners: Vec<usize> = (0..n).filter(|&i| !straight[i]).collect();
        let m = corners.len();
        for (k, &i0) in corners.iter().enumerate() {
            let i1 = corners[(k + 1) % m];
            let (a, b) = (c[i0], c[i1]);
            let d = [b[0] - a[0], b[1] - a[1]];
            let l = d[0].hypot(d[1]);
            let run = (1..(i1 + n - i0) % n).map(|j| (i0 + j) % n);
            if l > 1e-6
                && m >= 3
                && open(a, c[corners[(k + m - 1) % m]], b)
                && open(b, a, c[corners[(k + 2) % m]])
                && run.clone().all(|j| {
                    let e = [c[j][0] - a[0], c[j][1] - a[1]];
                    (d[0] * e[1] - d[1] * e[0]).abs() / l < 1.0
                })
            {
                for j in run {
                    graze[j] = true;
                }
            }
        }
        if graze.iter().filter(|g| !**g).count() >= 3 {
            c = c
                .into_iter()
                .zip(graze)
                .filter(|(_, g)| !g)
                .map(|(p, _)| p)
                .collect();
        }
    }
    loop {
        let n = c.len();
        if n < 4 {
            return c;
        }
        let mut done = true;
        for i in 0..n {
            let (p, q) = (c[i], c[(i + 1) % n]);
            let len = (q[0] - p[0]).hypot(q[1] - p[1]);
            if len >= nick {
                continue;
            }
            let (a, b) = (c[(i + n - 1) % n], c[(i + 2) % n]);
            // Where the line a→p meets the line b→q.
            let (d1, d2) = ([p[0] - a[0], p[1] - a[1]], [q[0] - b[0], q[1] - b[1]]);
            let den = d1[0] * d2[1] - d1[1] * d2[0];
            let (l1, l2) = (d1[0].hypot(d1[1]), d2[0].hypot(d2[1]));
            // Between two longer sides: a curve drawn in short pieces is all
            // short sides, and none of them is a nick.
            if l1 <= len || l2 <= len || den.abs() < 1e-3 * l1 * l2 {
                continue;
            }
            let t = ((b[0] - a[0]) * d2[1] - (b[1] - a[1]) * d2[0]) / den;
            let s = ((b[0] - a[0]) * d1[1] - (b[1] - a[1]) * d1[0]) / den;
            let x = [a[0] + d1[0] * t, a[1] + d1[1] * t];
            let mid = [(p[0] + q[0]) * 0.5, (p[1] + q[1]) * 0.5];
            // Ahead of both sides, and no further off than the side is long.
            if t < 1.0 || s < 1.0 || (x[0] - mid[0]).hypot(x[1] - mid[1]) > len.max(1.0) {
                continue;
            }
            // And only into a corner of 60° or more: a ray's tip or end closed
            // into a point would be rounded half way down its length. And of
            // 150° or less, a corner, not a bend in a curve.
            let (ua, ub) = ([-d1[0] / l1, -d1[1] / l1], [-d2[0] / l2, -d2[1] / l2]);
            let cos = ua[0] * ub[0] + ua[1] * ub[1];
            if !(-0.866..=WIDE).contains(&cos) {
                continue;
            }
            // And only where that corner's arc reaches back past the side:
            // the effect draws only where the picture is, so a corner closed
            // out over a gap, and rounded short of the side, would leave the
            // side's own corners sharp.
            let half = cos.clamp(-1.0, 1.0).acos() * 0.5;
            let reach = nick.min(l1 * t * 0.5).min(l2 * s * 0.5);
            let apex = reach * (1.0 - half.sin()) / half.cos();
            if apex < (x[0] - mid[0]).hypot(x[1] - mid[1]) {
                continue;
            }
            c[i] = x;
            c.remove((i + 1) % n);
            done = false;
            break;
        }
        if done {
            return c;
        }
    }
}

/// A closed contour with every corner rounded by `radius`, as a fillet: the
/// arc meets both sides `radius` from the corner, as Cavalry's Bevel does, and
/// no further than half way along the shorter one, so a thin piece is rounded
/// by what it has room for and no piece goes. At a right angle that is an arc
/// of `radius`; a wider corner's arc is larger, a sharper one's smaller.
///
/// A corner a cut has clipped into a side shorter than the radius (bars that
/// meet near it, as many do at the middle of a fan) is one corner, not two: it
/// is closed back into its point first ([`without_nicks`]), so it rounds by the
/// whole radius, as Cavalry's Bevel rounds it, rather than by half that side.
#[must_use]
pub fn filleted(contour: &[[f32; 2]], radius: f32) -> Vec<[f32; 2]> {
    if contour.len() < 3 || radius <= 0.0 {
        return contour.to_vec();
    }
    let contour = &without_nicks(contour, radius);
    let n = contour.len();
    if n < 3 {
        return contour.clone();
    }
    let mut out = Vec::with_capacity(n * 4);
    for i in 0..n {
        let p = contour[i];
        let (a, b) = (contour[(i + n - 1) % n], contour[(i + 1) % n]);
        let (la, lb) = (
            (a[0] - p[0]).hypot(a[1] - p[1]),
            (b[0] - p[0]).hypot(b[1] - p[1]),
        );
        if la < 1e-6 || lb < 1e-6 {
            continue;
        }
        let (ua, ub) = (
            [(a[0] - p[0]) / la, (a[1] - p[1]) / la],
            [(b[0] - p[0]) / lb, (b[1] - p[1]) / lb],
        );
        // Half the angle between the two sides.
        let half = (ua[0] * ub[0] + ua[1] * ub[1]).clamp(-1.0, 1.0).acos() * 0.5;
        if !(1e-4..=std::f32::consts::FRAC_PI_2 - 1e-3).contains(&half) {
            out.push(p);
            continue;
        }
        // The radius is how far along each side the arc starts, as Cavalry's
        // Bevel takes it: a wide corner rounds broadly, a sharp one tightly.
        let reach = radius.min(la.min(lb) * 0.5);
        let r = reach * half.tan();
        let bis = [ua[0] + ub[0], ua[1] + ub[1]];
        let bl = bis[0].hypot(bis[1]);
        let c = [
            p[0] + bis[0] / bl * r / half.sin(),
            p[1] + bis[1] / bl * r / half.sin(),
        ];
        let (s, e) = (
            [p[0] + ua[0] * reach, p[1] + ua[1] * reach],
            [p[0] + ub[0] * reach, p[1] + ub[1] * reach],
        );
        let a0 = (s[1] - c[1]).atan2(s[0] - c[0]);
        let mut sweep = (e[1] - c[1]).atan2(e[0] - c[0]) - a0;
        if sweep > std::f32::consts::PI {
            sweep -= std::f32::consts::TAU;
        } else if sweep < -std::f32::consts::PI {
            sweep += std::f32::consts::TAU;
        }
        // About a segment every 3 px of arc, at least one a 12th of a turn.
        let steps = ((sweep.abs() * r / 3.0).ceil() as usize)
            .max((sweep.abs() / 0.52).ceil() as usize)
            .clamp(1, 64);
        for k in 0..=steps {
            let t = a0 + sweep * k as f32 / steps as f32;
            out.push([c[0] + r * t.cos(), c[1] + r * t.sin()]);
        }
    }
    out
}
