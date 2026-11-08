//! Round corners worked out on a picture: a distance field round the alpha,
//! grown back by the radius.

/// The least of `(q - p)² + f(p)` for each cell q of a row, over the cells p
/// where `f` is finite (Felzenszwalb and Huttenlocher).
fn edt_1d(f: &[f32], out: &mut [f32], v: &mut [usize], z: &mut [f32]) {
    let n = f.len();
    let mut k = 0usize;
    let mut seeded = false;
    for q in 0..n {
        if !f[q].is_finite() {
            continue;
        }
        if !seeded {
            v[0] = q;
            z[0] = f32::NEG_INFINITY;
            z[1] = f32::INFINITY;
            seeded = true;
            continue;
        }
        loop {
            let p = v[k];
            let s =
                ((f[q] + (q * q) as f32) - (f[p] + (p * p) as f32)) / (2.0 * (q as f32 - p as f32));
            if s <= z[k] && k > 0 {
                k -= 1;
                continue;
            }
            k += 1;
            v[k] = q;
            z[k] = s;
            z[k + 1] = f32::INFINITY;
            break;
        }
    }
    if !seeded {
        out.fill(f32::INFINITY);
        return;
    }
    let mut j = 0;
    for (q, o) in out.iter_mut().enumerate() {
        while z[j + 1] < q as f32 {
            j += 1;
        }
        let d = q as f32 - v[j] as f32;
        *o = d * d + f[v[j]];
    }
}

/// For each pixel, the least of `|p - q|² + f(q)` over every pixel q, raster
/// px: the lower envelope of a paraboloid on each pixel, row then column.
fn envelope(mut d: Vec<f32>, w: u32, h: u32) -> Vec<f32> {
    let (w, h) = (w as usize, h as usize);
    let m = w.max(h);
    let (mut f, mut out, mut v, mut z) = (
        vec![0.0; m],
        vec![0.0; m],
        vec![0usize; m],
        vec![0.0; m + 1],
    );
    for x in 0..w {
        for y in 0..h {
            f[y] = d[y * w + x];
        }
        edt_1d(&f[..h], &mut out[..h], &mut v, &mut z);
        for y in 0..h {
            d[y * w + x] = out[y];
        }
    }
    for y in 0..h {
        f[..w].copy_from_slice(&d[y * w..(y + 1) * w]);
        edt_1d(&f[..w], &mut out[..w], &mut v, &mut z);
        d[y * w..(y + 1) * w].copy_from_slice(&out[..w]);
    }
    d
}

/// Distance from each pixel's centre to the nearest pixel `seed` says yes to,
/// raster px.
fn distance_to(seed: impl Fn(usize) -> bool, w: u32, h: u32) -> Vec<f32> {
    let d = (0..(w * h) as usize)
        .map(|i| if seed(i) { 0.0 } else { f32::INFINITY })
        .collect();
    let mut d = envelope(d, w, h);
    d.iter_mut().for_each(|v| *v = v.sqrt());
    d
}

/// The pieces of `solid`, 8-connected: each pixel's piece number, or
/// `u32::MAX` outside, and how many pieces there are.
fn pieces(solid: &[bool], w: u32, h: u32) -> (Vec<u32>, usize) {
    let (w, h) = (w as usize, h as usize);
    let mut label = vec![u32::MAX; w * h];
    let mut stack = Vec::new();
    let mut n = 0u32;
    for start in 0..w * h {
        if !solid[start] || label[start] != u32::MAX {
            continue;
        }
        label[start] = n;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = ((i % w) as i64, (i / w) as i64);
            for (dx, dy) in [
                (-1, -1),
                (0, -1),
                (1, -1),
                (-1, 0),
                (1, 0),
                (-1, 1),
                (0, 1),
                (1, 1),
            ] {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                if solid[j] && label[j] == u32::MAX {
                    label[j] = n;
                    stack.push(j);
                }
            }
        }
        n += 1;
    }
    (label, n as usize)
}

/// Coverage `alpha` with its outward corners rounded to `radius` raster px:
/// what a disc of that radius reaches while staying inside (an opening). A
/// piece too thin for the disc is rounded by half its width instead, as a
/// fillet stops at the sides it joins, so no piece goes. It never grows, so
/// the colours under it stay.
#[must_use]
pub fn rounded(alpha: &[f32], w: u32, h: u32, radius: f32) -> Vec<f32> {
    if radius <= 0.5 {
        return alpha.to_vec();
    }
    let solid: Vec<bool> = alpha.iter().map(|&a| a >= 0.5).collect();
    // How far each pixel's centre is from the edge, plus half a pixel.
    let inside = distance_to(|i| !solid[i], w, h);
    let (label, count) = pieces(&solid, w, h);
    // Each piece's disc: the radius, or the widest that fits in it.
    let mut widest = vec![0.0f32; count];
    for (i, &l) in label.iter().enumerate() {
        if let Some(m) = widest.get_mut(l as usize) {
            *m = m.max(inside[i]);
        }
    }
    let disc: Vec<f32> = widest.iter().map(|&m| radius.min(m - 0.5)).collect();
    // The centres the disc fits on, as -r²: the envelope is below 0 wherever
    // one reaches. The extra pixel makes up for centres that start on
    // the grid, past where the true ones do, and for the half pixel between an
    // edge pixel's centre and the edge.
    let centres = (0..alpha.len())
        .map(|i| match disc.get(label[i] as usize) {
            Some(&r) if inside[i] - 0.5 >= r => -(r + 1.0).powi(2),
            _ => f32::INFINITY,
        })
        .collect();
    let reach = envelope(centres, w, h);
    // How far past the nearest disc's rim, near enough: r² grows by 2r a px.
    (0..alpha.len())
        .map(|i| {
            let r = disc
                .get(label[i] as usize)
                .copied()
                .unwrap_or(radius)
                .max(0.5);
            alpha[i].min((0.5 - reach[i] / (2.0 * (r + 1.0))).clamp(0.0, 1.0))
        })
        .collect()
}
