//! The app icon, drawn in code: a slightly tilted acorn inside a ring of
//! sunburst-coloured segments. The ring doubles as a progress indicator, so
//! the very same drawing is used for the static icon files, the window icon
//! and the animated macOS Dock tile while scanning.
//!
//! Coordinates are on a 100×100 canvas, like the design mock-ups.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use tiny_skia::{Color, FillRule, LineCap, Paint, Path, PathBuilder, Pixmap, Stroke, Transform};

/// What the outer ring shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ring {
    /// Default look: every segment lit.
    Full,
    /// Scan progress, 0.0 – 1.0.
    Progress(f32),
    /// Progress unknown (folder scans): a lit arc chasing around.
    Spinner(f32),
}

#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub ring: Ring,
    /// Acorn rotation wobble in degrees (added to the resting −16° tilt).
    pub wobble: f32,
    /// Vertical hop in canvas units (negative = up).
    pub hop: f32,
}

impl Frame {
    pub const IDLE: Frame = Frame { ring: Ring::Full, wobble: 0.0, hop: 0.0 };
}

const BG: [u8; 3] = [0x2a, 0x1c, 0x3c];
const DIM: [u8; 3] = [0x4a, 0x3a, 0x60];
const WHEEL: [[u8; 3]; 12] = [
    [0x39, 0x6b, 0xff],
    [0x8b, 0x4d, 0xff],
    [0xd9, 0x46, 0xef],
    [0xff, 0x4f, 0x9a],
    [0xff, 0x70, 0x5a],
    [0xff, 0xb3, 0x47],
    [0xff, 0xe4, 0x5c],
    [0xb8, 0xf0, 0x4a],
    [0x4c, 0xe0, 0x6a],
    [0x2f, 0xd8, 0xc0],
    [0x3a, 0xc8, 0xff],
    [0x5a, 0x7b, 0xff],
];
const NUT: [u8; 3] = [0xe8, 0xa0, 0x4c];
const NUT_SHADE: [u8; 3] = [0xc7, 0x7a, 0x2c];
const NUT_HI: [u8; 3] = [0xf6, 0xc2, 0x7a];
const CAP: [u8; 3] = [0x8a, 0x55, 0x2e];
const CAP_DARK: [u8; 3] = [0x6b, 0x3f, 0x1f];
const CAP_SCALE: [u8; 3] = [0xa8, 0x6c, 0x3c];

fn paint(c: [u8; 3], alpha: u8) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(Color::from_rgba8(c[0], c[1], c[2], alpha));
    p.anti_alias = true;
    p
}

fn stroke(width: f32) -> Stroke {
    Stroke { width, line_cap: LineCap::Round, ..Stroke::default() }
}

/// Annular sector from angle `a0` to `a1` (radians, 0 = 12 o'clock, clockwise).
fn sector(r0: f32, r1: f32, a0: f32, a1: f32) -> Option<Path> {
    let pt = |r: f32, a: f32| (50.0 + r * a.sin(), 50.0 - r * a.cos());
    let steps = (((a1 - a0) * 24.0).ceil() as usize).max(2);
    let mut pb = PathBuilder::new();
    let (x, y) = pt(r1, a0);
    pb.move_to(x, y);
    for i in 1..=steps {
        let (x, y) = pt(r1, a0 + (a1 - a0) * i as f32 / steps as f32);
        pb.line_to(x, y);
    }
    for i in (0..=steps).rev() {
        let (x, y) = pt(r0, a0 + (a1 - a0) * i as f32 / steps as f32);
        pb.line_to(x, y);
    }
    pb.close();
    pb.finish()
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    let k = 0.5523 * r; // cubic approximation of a quarter circle
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

fn draw_ring(pm: &mut Pixmap, ring: Ring, t: Transform) {
    const N: usize = 12;
    const GAP: f32 = 0.07;
    let tau = std::f32::consts::TAU;
    for i in 0..N {
        let a0 = i as f32 / N as f32 * tau + GAP / 2.0;
        let a1 = (i + 1) as f32 / N as f32 * tau - GAP / 2.0;
        if let Some(p) = sector(31.0, 41.0, a0, a1) {
            pm.fill_path(&p, &paint(DIM, 255), FillRule::Winding, t, None);
        }
        let (fill, alpha) = match ring {
            Ring::Full => (1.0, 255),
            Ring::Progress(p) => ((p * N as f32 - i as f32).clamp(0.0, 1.0), 255),
            Ring::Spinner(phase) => {
                // a soft 4-segment comet travelling clockwise
                let head = phase.rem_euclid(1.0) * N as f32;
                let d = (head - i as f32).rem_euclid(N as f32);
                let a = if d < 4.0 { 1.0 - d / 4.0 } else { 0.0 };
                (if a > 0.0 { 1.0 } else { 0.0 }, (a * 255.0) as u8)
            }
        };
        if fill > 0.0 && alpha > 0 {
            if let Some(p) = sector(31.0, 41.0, a0, a0 + (a1 - a0) * fill) {
                pm.fill_path(&p, &paint(WHEEL[i], alpha), FillRule::Winding, t, None);
            }
        }
    }
}

fn draw_acorn(pm: &mut Pixmap, t: Transform) {
    let (w, h, top, cw, ch, stem) = (15.0f32, 24.0f32, 50.0f32, 19.0f32, 16.0f32, 6.0f32);
    let bot = top + h;
    let fill = |pm: &mut Pixmap, p: Option<Path>, c: [u8; 3], a: u8| {
        if let Some(p) = p {
            pm.fill_path(&p, &paint(c, a), FillRule::Winding, t, None);
        }
    };
    let line = |pm: &mut Pixmap, p: Option<Path>, c: [u8; 3], width: f32| {
        if let Some(p) = p {
            pm.stroke_path(&p, &paint(c, 255), &stroke(width), t, None);
        }
    };
    // nut
    let mut pb = PathBuilder::new();
    pb.move_to(50.0 - w, top);
    pb.quad_to(50.0 - w * 1.02, top + h * 0.78, 50.0, bot);
    pb.quad_to(50.0 + w * 1.02, top + h * 0.78, 50.0 + w, top);
    pb.close();
    fill(pm, pb.finish(), NUT, 255);
    // right-side shade
    let mut pb = PathBuilder::new();
    pb.move_to(50.0, top);
    pb.line_to(50.0 + w, top);
    pb.quad_to(50.0 + w * 1.02, top + h * 0.78, 50.0, bot);
    pb.close();
    fill(pm, pb.finish(), NUT_SHADE, 115);
    // highlight
    let mut pb = PathBuilder::new();
    pb.move_to(50.0 - w * 0.7, top + h * 0.22);
    pb.quad_to(50.0 - w * 0.78, top + h * 0.55, 50.0 - w * 0.4, top + h * 0.75);
    line(pm, pb.finish(), NUT_HI, 2.6);
    // tip
    let mut pb = PathBuilder::new();
    pb.move_to(47.0, bot - 2.0);
    pb.line_to(50.0, bot + 3.0);
    pb.line_to(53.0, bot - 2.0);
    pb.close();
    fill(pm, pb.finish(), NUT_SHADE, 255);
    // cap
    let ct = top - ch;
    let mut pb = PathBuilder::new();
    pb.move_to(50.0 - cw, top + 1.0);
    pb.quad_to(50.0 - cw * 1.03, ct, 50.0, ct);
    pb.quad_to(50.0 + cw * 1.03, ct, 50.0 + cw, top + 1.0);
    pb.quad_to(50.0, top + 5.0, 50.0 - cw, top + 1.0);
    pb.close();
    fill(pm, pb.finish(), CAP, 255);
    // cap scales: three staggered rows of little arches
    for r in 0..3 {
        let cnt = 6 - r;
        let span = cw * 2.0 - 8.0 - r as f32 * 6.0;
        let step = span / cnt as f32;
        let yy = top - 2.0 - r as f32 * ch * 0.3;
        for k in 0..cnt {
            let x = 50.0 - span / 2.0 + k as f32 * step;
            let mut pb = PathBuilder::new();
            pb.move_to(x, yy);
            pb.quad_to(x + step / 2.0, yy - 2.6, x + step, yy);
            line(pm, pb.finish(), CAP_SCALE, 1.3);
        }
    }
    // stem
    let mut pb = PathBuilder::new();
    pb.move_to(50.0, ct);
    pb.quad_to(51.0, ct - 7.0, 50.0 + stem, ct - 9.0);
    line(pm, pb.finish(), CAP_DARK, 3.2);
}

/// Renders the icon. `inset` shrinks the tile inside the canvas (macOS icons
/// use a 824/1024 body with transparent margins; window icons use the full size).
pub fn render(size: u32, frame: Frame, inset: bool) -> Pixmap {
    let mut pm = Pixmap::new(size, size).expect("icon size");
    let body = if inset { 824.0 / 1024.0 } else { 1.0 };
    let s = size as f32 * body / 100.0;
    let off = size as f32 * (1.0 - body) / 2.0;
    let base = Transform::from_translate(off, off).pre_scale(s, s);
    // The tile goes edge to edge of the body; radius follows the macOS squircle ratio.
    if let Some(p) = rounded_rect(0.0, 0.0, 100.0, 100.0, 22.5) {
        pm.fill_path(&p, &paint(BG, 255), FillRule::Winding, base, None);
    }
    draw_ring(&mut pm, frame.ring, base);
    let acorn = base
        .pre_concat(Transform::from_translate(0.0, frame.hop))
        .pre_concat(Transform::from_rotate_at(-16.0 + frame.wobble, 50.0, 58.0));
    draw_acorn(&mut pm, acorn);
    pm
}

/// Straight (non-premultiplied) RGBA bytes, as window-icon APIs expect.
pub fn rgba(pm: &Pixmap) -> Vec<u8> {
    pm.pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

pub fn png(size: u32, frame: Frame, inset: bool) -> Vec<u8> {
    render(size, frame, inset).encode_png().unwrap_or_default()
}

/// Animation frame for a scan in progress (`fraction` is `None` when unknown).
pub fn scanning_frame(fraction: Option<f32>, time: f32) -> Frame {
    Frame {
        ring: match fraction {
            Some(f) => Ring::Progress(f),
            None => Ring::Spinner(time * 0.6),
        },
        wobble: (time * 4.0).sin() * 3.0,
        hop: 0.0,
    }
}

/// Little hop when a scan completes (`t` seconds since completion, ~1.2 s long).
pub fn done_frame(t: f32) -> Frame {
    let k = (1.0 - t / 1.2).max(0.0);
    Frame { ring: Ring::Full, wobble: 0.0, hop: -((t * 9.0).sin().abs() * 3.0 * k) }
}

/// Writes every icon asset into `dir`: PNGs, a Windows `.ico` and, on macOS,
/// an `.icns` built with `iconutil`.
pub fn write_assets(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for s in [16u32, 32, 48, 64, 128, 256, 512, 1024] {
        std::fs::write(dir.join(format!("icon-{s}.png")), png(s, Frame::IDLE, false))?;
    }
    // .ico with embedded PNG images (supported since Windows Vista)
    let sizes = [16u32, 24, 32, 48, 64, 128, 256];
    let images: Vec<Vec<u8>> = sizes.iter().map(|s| png(*s, Frame::IDLE, false)).collect();
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (s, img) in sizes.iter().zip(&images) {
        let b = if *s >= 256 { 0 } else { *s as u8 };
        ico.extend_from_slice(&[b, b, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(img.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in &images {
        ico.extend_from_slice(img);
    }
    std::fs::write(dir.join("SquirrelDisk.ico"), ico)?;
    // macOS iconset → icns
    let set = dir.join("SquirrelDisk.iconset");
    std::fs::create_dir_all(&set)?;
    for s in [16u32, 32, 128, 256, 512] {
        std::fs::write(set.join(format!("icon_{s}x{s}.png")), png(s, Frame::IDLE, true))?;
        std::fs::write(set.join(format!("icon_{s}x{s}@2x.png")), png(s * 2, Frame::IDLE, true))?;
    }
    if cfg!(target_os = "macos") {
        let ok = std::process::Command::new("iconutil")
            .arg("-c")
            .arg("icns")
            .arg(&set)
            .arg("-o")
            .arg(dir.join("SquirrelDisk.icns"))
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            let _ = std::fs::remove_dir_all(&set);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_all_states() {
        for f in [Frame::IDLE, scanning_frame(Some(0.4), 1.0), scanning_frame(None, 2.0), done_frame(0.3)] {
            let pm = render(64, f, true);
            assert_eq!(pm.width(), 64);
            // the centre (acorn) is opaque, the inset corner is transparent
            assert_eq!(pm.pixel(32, 40).unwrap().alpha(), 255);
            assert_eq!(pm.pixel(0, 0).unwrap().alpha(), 0);
        }
    }
}

