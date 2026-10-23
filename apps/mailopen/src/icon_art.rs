// The app icon, drawn in code (original artwork, same design as packaging/macos/make-icon.swift):
// a blue rounded square with a vertical gradient, a white envelope, its flap, an orange
// "unread" dot. Shared by the window icon (src/icon.rs) and the .exe icon (build.rs includes
// this file), so it has no dependencies.

/// Straight-alpha RGBA pixels of the icon, `size` × `size`.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    let n = size.max(1);
    let s = n as f32;
    // About one pixel, in unit coordinates: the anti-aliasing width.
    let aa = 1.0 / s;
    // Small icons get thicker strokes and a bigger dot, or they vanish.
    let small = (32.0 - s).clamp(0.0, 16.0) / 16.0;
    let stroke = 0.045 + 0.035 * small;
    let dot_r = 0.07 + 0.03 * small;
    let inset = 0.06 - 0.04 * small;
    let mut out = vec![0u8; (n as usize) * (n as usize) * 4];
    for (i, px) in out.chunks_exact_mut(4).enumerate() {
        let x = (i % n as usize) as f32 + 0.5;
        let y = (i / n as usize) as f32 + 0.5;
        let (u, v) = (x / s, y / s);
        let mut c = [0.0f32; 4];
        // Background: rounded square, light blue at the top to navy at the bottom.
        let half = 0.5 - inset;
        let d = rounded_rect(u, v, 0.5, 0.5, half, half, 0.2);
        let t = ((v - inset) / (1.0 - 2.0 * inset)).clamp(0.0, 1.0);
        let lerp = |a: f32, b: f32| a + (b - a) * t;
        let bg = [lerp(0.21, 0.11), lerp(0.47, 0.23), lerp(0.93, 0.39)];
        over(&mut c, bg, coverage(d, aa));
        // Envelope.
        let (w, h) = (0.56 + 0.06 * small, 0.40 + 0.04 * small);
        let (x0, y0) = (0.5 - w / 2.0, 0.5 - h / 2.0);
        let d = rounded_rect(u, v, 0.5, 0.5, w / 2.0, h / 2.0, 0.04);
        over(&mut c, [1.0, 1.0, 1.0], coverage(d, aa));
        // Flap: a V from the top corners to just below the middle.
        let flap = [0.13, 0.28, 0.47];
        let (ax, ay) = (x0 + 0.02 + stroke / 2.0, y0 + 0.02 + stroke / 2.0);
        let (bx, by) = (0.5, y0 + h * 0.58);
        let (cx, cy) = (x0 + w - 0.02 - stroke / 2.0, ay);
        let d = segment(u, v, ax, ay, bx, by).min(segment(u, v, bx, by, cx, cy)) - stroke / 2.0;
        // Only inside the envelope.
        let inside = coverage(rounded_rect(u, v, 0.5, 0.5, w / 2.0, h / 2.0, 0.04), aa);
        over(&mut c, flap, coverage(d, aa) * inside);
        // Unread dot at the envelope's top right corner.
        let d = ((u - (x0 + w)).powi(2) + (v - y0).powi(2)).sqrt() - dot_r;
        over(&mut c, [0.96, 0.62, 0.04], coverage(d, aa));
        for (dst, src) in px.iter_mut().zip(c.iter()) {
            *dst = (src.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    out
}

/// Signed distance to a rounded rectangle centred at (cx, cy) with half sizes (hw, hh).
fn rounded_rect(u: f32, v: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let qx = (u - cx).abs() - hw + r;
    let qy = (v - cy).abs() - hh + r;
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - r
}

/// Distance to the segment from (ax, ay) to (bx, by).
fn segment(u: f32, v: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (px, py) = (u - ax, v - ay);
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        ((px * dx + py * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    ((px - dx * t).powi(2) + (py - dy * t).powi(2)).sqrt()
}

/// How much of the pixel a shape covers, from its signed distance.
fn coverage(d: f32, aa: f32) -> f32 {
    (0.5 - d / aa).clamp(0.0, 1.0)
}

/// Paint `rgb` with opacity `a` over the straight-alpha colour `c`.
fn over(c: &mut [f32; 4], rgb: [f32; 3], a: f32) {
    if a <= 0.0 {
        return;
    }
    let [r, g, b, ca] = *c;
    let out_a = a + ca * (1.0 - a);
    if out_a <= 0.0 {
        return;
    }
    let mix = |src: f32, dst: f32| (src * a + dst * ca * (1.0 - a)) / out_a;
    let [sr, sg, sb] = rgb;
    *c = [mix(sr, r), mix(sg, g), mix(sb, b), out_a];
}

/// A Windows `.ico` with the icon at the usual sizes (32-bit BMP entries).
#[allow(dead_code)]
pub fn ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<(u32, Vec<u8>)> = sizes
        .iter()
        .map(|&s| (s, bmp_entry(s, &icon_rgba(s))))
        .collect();
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (s, data) in &images {
        let dim = if *s >= 256 { 0u8 } else { *s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in &images {
        out.extend_from_slice(data);
    }
    out
}

/// One `.ico` entry: BITMAPINFOHEADER, BGRA rows bottom-up, an empty AND mask.
fn bmp_entry(size: u32, rgba: &[u8]) -> Vec<u8> {
    let n = size as usize;
    let mask_row = n.div_ceil(32) * 4;
    let mut out = Vec::with_capacity(40 + n * n * 4 + mask_row * n);
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(size as i32).to_le_bytes());
    out.extend_from_slice(&((size * 2) as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&((n * n * 4 + mask_row * n) as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);
    for row in rgba.chunks_exact(n * 4).rev() {
        for p in row.chunks_exact(4) {
            if let [r, g, b, a] = p {
                out.extend_from_slice(&[*b, *g, *r, *a]);
            }
        }
    }
    out.extend(std::iter::repeat_n(0u8, mask_row * n));
    out
}
