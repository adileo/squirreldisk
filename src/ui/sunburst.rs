//! Sunburst layout, keyed animation and hit testing.
//!
//! The layout is recomputed only when the tree/view/size changes. Every segment
//! is identified by a stable key (node id, or parent id + flag for "other"
//! groups) so that when the view changes each segment smoothly morphs from its
//! old angles/depth to the new ones: zooming in literally expands the clicked
//! slice into the center disc.

use super::theme::{lerp_color, Theme};
use crate::tree::{Kind, Tree, NONE};
use eframe::egui::{Color32, Pos2, Vec2};
use std::collections::HashMap;
use std::f32::consts::TAU;

pub const GROUP_FLAG: u64 = 1 << 40;
/// Key of the virtual "free space" slice.
pub const FREE_KEY: u64 = u64::MAX - 1;
/// Minimum arc length (points, measured at mid-ring) for a slice to be drawn;
/// smaller ones are folded into a "smaller items" wedge. Inner rings have
/// shorter arcs, so they fold earlier than outer rings.
const MIN_PX: f32 = 4.0;
const MAX_SEGMENTS: usize = 14_000;

/// What the center of the chart represents.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct View {
    pub node: u32,
    /// When > 0, only children from this index (sorted by size) are shown:
    /// the content of an "other" group.
    pub skip: usize,
}

impl View {
    pub fn node(node: u32) -> Self {
        View { node, skip: 0 }
    }
    pub fn key(&self) -> u64 {
        if self.skip > 0 { group_key(self.node, self.skip) } else { self.node as u64 }
    }
}

pub fn group_key(parent: u32, skip: usize) -> u64 {
    GROUP_FLAG | ((skip as u64) << 42) | parent as u64
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SegKind {
    Node(Kind),
    /// Aggregate of the children of `parent` from index `skip` on.
    Group { parent: u32, skip: usize, count: usize },
    /// Unused space on the volume (only at the root of a disk scan).
    Free,
}

#[derive(Clone, Debug)]
pub struct Seg {
    pub key: u64,
    pub node: u32,
    pub kind: SegKind,
    pub a0: f32,
    pub a1: f32,
    pub depth: u8,
    pub color: Color32,
    pub size: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub center: Pos2,
    /// Radius of the central disc.
    pub r0: f32,
    /// Width of each ring.
    pub ring: f32,
}

impl Geometry {
    pub fn new(center: Pos2, radius: f32, rings: usize) -> Self {
        let r0 = radius * 0.2;
        let ring = (radius - r0) / rings as f32;
        Geometry { center, r0, ring }
    }
    /// Inner/outer radius for a (possibly fractional) depth.
    pub fn radii(&self, d: f32) -> (f32, f32) {
        let d = d.max(0.0);
        let inner = self.r0 * d.min(1.0) + (d - 1.0).max(0.0) * self.ring;
        let thick = self.r0 + (self.ring - self.r0) * d.min(1.0);
        (inner, inner + thick)
    }
    pub fn polar(&self, p: Pos2) -> (f32, f32) {
        let v = p - self.center;
        let r = v.length();
        let a = v.x.atan2(-v.y).rem_euclid(TAU);
        (r, a)
    }
    pub fn point(&self, r: f32, a: f32) -> Pos2 {
        self.center + Vec2::new(a.sin(), -a.cos()) * r
    }
}

pub fn view_children(tree: &Tree, view: View) -> (Vec<u32>, u64) {
    let mut kids = tree.sorted_children(view.node);
    if view.skip > 0 {
        kids = kids.split_off(view.skip.min(kids.len()));
        let total = kids.iter().map(|k| tree.get(*k).size).sum();
        (kids, total)
    } else {
        let n = tree.get(view.node);
        let sum: u64 = kids.iter().map(|k| tree.get(*k).size).sum();
        (kids, n.size.max(sum))
    }
}

pub fn view_size(tree: &Tree, view: View) -> u64 {
    view_children(tree, view).1
}

/// Computes the target layout for a view. `free` > 0 adds a free-space slice
/// at the end of the first ring (used at the root of a volume).
pub fn layout(tree: &Tree, view: View, geo: &Geometry, rings: usize, theme: &Theme, free: u64) -> Vec<Seg> {
    let mut out = Vec::with_capacity(1024);
    if !tree.is_alive(view.node) {
        return out;
    }
    let (kids, total) = view_children(tree, view);
    let center_kind = if view.skip > 0 {
        SegKind::Group { parent: view.node, skip: view.skip, count: kids.len() }
    } else {
        SegKind::Node(tree.get(view.node).kind)
    };
    out.push(Seg { key: view.key(), node: view.node, kind: center_kind, a0: 0.0, a1: TAU, depth: 0, color: theme.surface, size: total });
    if total == 0 {
        return out;
    }
    let whole = total + free;
    if free > 0 {
        let a0 = TAU * (total as f64 / whole as f64) as f32;
        let color = lerp_color(theme.bg_top, theme.text_faint, 0.32);
        out.push(Seg { key: FREE_KEY, node: view.node, kind: SegKind::Free, a0, a1: TAU, depth: 1, color, size: free });
    }
    let mut stack: Vec<(Vec<u32>, usize, u64, f32, f32, u8)> = vec![(kids, 0, whole, 0.0, TAU, 1)];
    // Iterative DFS: (children, skip offset, parent size, a0, span, depth)
    while let Some((items, base_skip, parent_size, a0, span, depth)) = stack.pop() {
        if out.len() > MAX_SEGMENTS {
            break;
        }
        let (r_in, r_out) = geo.radii(depth as f32);
        let min_angle = MIN_PX / ((r_in + r_out) * 0.5).max(1.0);
        let mut a = a0;
        let parent_node = items.first().map(|c| tree.get(*c).parent).unwrap_or(NONE);
        for (i, &c) in items.iter().enumerate() {
            let n = tree.get(c);
            let ang = span * (n.size as f64 / parent_size as f64) as f32;
            if ang < min_angle && i + 1 < items.len() {
                // group the tail
                let rest: u64 = items[i..].iter().map(|k| tree.get(*k).size).sum();
                let ang_rest = span * (rest as f64 / parent_size as f64) as f32;
                if ang_rest >= min_angle * 0.6 {
                    let skip = base_skip + i;
                    let mid = (a + ang_rest * 0.5) / TAU;
                    let _ = mid;
                    out.push(Seg {
                        key: group_key(parent_node, skip),
                        node: parent_node,
                        kind: SegKind::Group { parent: parent_node, skip, count: items.len() - i },
                        a0: a,
                        a1: a + ang_rest,
                        depth,
                        color: theme.aggregate_color(),
                        size: rest,
                    });
                }
                break;
            }
            if ang < min_angle * 0.25 {
                break;
            }
            let mid = (a + ang * 0.5) / TAU;
            let color = match n.kind {
                Kind::Dir => theme.segment_color(mid, depth as usize, false),
                Kind::File => theme.segment_color(mid, depth as usize, true),
                Kind::SmallFiles => theme.aggregate_color(),
                Kind::Hidden => lerp_color(theme.neutral, theme.bg_bottom, 0.35),
                Kind::Mount | Kind::Link => theme.text_faint,
            };
            out.push(Seg { key: c as u64, node: c, kind: SegKind::Node(n.kind), a0: a, a1: a + ang, depth, color, size: n.size });
            if n.kind == Kind::Dir && (depth as usize) < rings && ang > min_angle * 1.5 && n.first_child != NONE {
                stack.push((tree.sorted_children(c), 0, n.size.max(1), a, ang, depth + 1));
            }
            a += ang;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Animation

#[derive(Clone, Debug)]
pub struct Anim {
    pub key: u64,
    pub node: u32,
    pub kind: SegKind,
    pub a0: f32,
    pub a1: f32,
    pub d: f32,
    pub color: Color32,
    pub alpha: f32,
    pub grow: f32,
    pub hover: f32,
    pub alive: bool,
    pub size: u64,
    t_a0: f32,
    t_a1: f32,
    t_d: f32,
    t_color: Color32,
}

#[derive(Default)]
pub struct Animator {
    pub anims: HashMap<u64, Anim>,
    /// Draw order (inner rings first).
    pub order: Vec<u64>,
    pub settled: bool,
}

impl Animator {
    pub fn set_target(&mut self, segs: &[Seg], intro: bool) {
        for a in self.anims.values_mut() {
            a.alive = false;
        }
        self.order.clear();
        for s in segs {
            self.order.push(s.key);
            match self.anims.get_mut(&s.key) {
                Some(a) => {
                    a.alive = true;
                    a.kind = s.kind;
                    a.node = s.node;
                    a.size = s.size;
                    a.t_a0 = s.a0;
                    a.t_a1 = s.a1;
                    a.t_d = s.depth as f32;
                    a.t_color = s.color;
                }
                None => {
                    let delay = if intro { -0.12 * s.depth as f32 - 0.25 * (s.a0 / TAU) } else { 0.0 };
                    self.anims.insert(
                        s.key,
                        Anim {
                            key: s.key,
                            node: s.node,
                            kind: s.kind,
                            a0: s.a0,
                            a1: s.a1,
                            d: s.depth as f32,
                            color: s.color,
                            alpha: 0.0,
                            grow: delay,
                            hover: 0.0,
                            alive: true,
                            size: s.size,
                            t_a0: s.a0,
                            t_a1: s.a1,
                            t_d: s.depth as f32,
                            t_color: s.color,
                        },
                    );
                }
            }
        }
        self.settled = false;
    }

    /// Advances the animation. Returns true while something is still moving.
    pub fn step(&mut self, dt: f32, hovered: Option<u64>) -> bool {
        let k = 1.0 - (-dt * 11.0).exp();
        let kh = 1.0 - (-dt * 18.0).exp();
        let mut moving = false;
        self.anims.retain(|_, a| a.alive || a.alpha > 0.01);
        for a in self.anims.values_mut() {
            let h_target = if Some(a.key) == hovered { 1.0 } else { 0.0 };
            a.hover += (h_target - a.hover) * kh;
            if a.alive {
                a.a0 += (a.t_a0 - a.a0) * k;
                a.a1 += (a.t_a1 - a.a1) * k;
                a.d += (a.t_d - a.d) * k;
                a.color = lerp_color(a.color, a.t_color, k * 1.2);
                a.alpha += (1.0 - a.alpha) * k;
                a.grow = (a.grow + dt * 2.6).min(1.0);
                if (a.t_a0 - a.a0).abs() > 1e-4 || (a.t_a1 - a.a1).abs() > 1e-4 || (a.t_d - a.d).abs() > 1e-3 || a.alpha < 0.995 || a.grow < 1.0 {
                    moving = true;
                }
            } else {
                a.alpha -= a.alpha * k * 1.4 + dt * 0.5;
                a.alpha = a.alpha.max(0.0);
                moving = true;
            }
            if (a.hover - h_target).abs() > 0.01 {
                moving = true;
            }
        }
        self.settled = !moving;
        moving
    }

    /// Segments to draw (alive first in order, then fading ones).
    pub fn draw_list(&self) -> Vec<&Anim> {
        let mut v: Vec<&Anim> = self.anims.values().filter(|a| !a.alive && a.alpha > 0.01).collect();
        v.extend(self.order.iter().filter_map(|k| self.anims.get(k)));
        v
    }

    pub fn hit(&self, geo: &Geometry, p: Pos2) -> Option<&Anim> {
        let (r, ang) = geo.polar(p);
        for key in self.order.iter().rev() {
            let Some(a) = self.anims.get(key) else { continue };
            if a.alpha < 0.3 {
                continue;
            }
            let (r0, r1) = geo.radii(a.d);
            let r1 = r0 + (r1 - r0) * ease(a.grow);
            if r < r0 || r > r1 {
                continue;
            }
            if a.d < 0.5 || (ang >= a.a0 && ang < a.a1) {
                return Some(a);
            }
        }
        None
    }
}

pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

// ---------------------------------------------------------------------------
// Geometry generation shared by the GL and CPU paths.

/// One vertex: pos(2) color(4) polar(4: da0, da1, dr0, dr1) params(4: hover, flags, radius, seed)
pub const VERT_FLOATS: usize = 14;

pub const FLAG_COLLECTED: f32 = 1.0;
pub const FLAG_DRAGGED: f32 = 2.0;
pub const FLAG_CENTER: f32 = 4.0;
pub const FLAG_AGGREGATE: f32 = 8.0;
pub const FLAG_SCANNING: f32 = 16.0;

pub struct SegStyle {
    pub flags: f32,
    pub lift: f32,
}

/// Tessellates annular sectors into a triangle list (interleaved floats).
pub fn build_vertices(
    anims: &[&Anim],
    geo: &Geometry,
    mut style: impl FnMut(&Anim) -> SegStyle,
    out: &mut Vec<f32>,
    indices: &mut Vec<u32>,
) {
    out.clear();
    indices.clear();
    for a in anims {
        let st = style(a);
        let (mut r0, mut r1) = geo.radii(a.d);
        let g = ease(a.grow);
        if g <= 0.0 {
            continue;
        }
        r1 = r0 + (r1 - r0) * g;
        let push = st.lift;
        if a.d > 0.5 {
            r1 += push;
            r0 += push * 0.3;
        }
        let (a0, a1) = if a.d < 0.5 { (0.0, TAU) } else { (a.a0, a.a1) };
        let span = (a1 - a0).max(0.0);
        if span <= 0.0 || r1 <= r0 {
            continue;
        }
        let steps = ((span * r1 / 4.0).ceil() as usize).clamp(1, 256);
        let c = a.color;
        let alpha = a.alpha;
        let col = [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0, alpha];
        let seed = ((a.key.wrapping_mul(2654435761)) % 1000) as f32 / 1000.0;
        let base = (out.len() / VERT_FLOATS) as u32;
        let center = a.d < 0.5;
        let flags = st.flags + if center { FLAG_CENTER } else { 0.0 };
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let ang = a0 + span * t;
            let (s, co) = ang.sin_cos();
            for (r, dr0, dr1) in [(r0, 0.0, r1 - r0), (r1, r1 - r0, 0.0)] {
                let (da0, da1) = if center { (1e3, 1e3) } else { (ang - a0, a1 - ang) };
                let (dr0, dr1) = if center { (r1 * 2.0, r1 - r) } else { (dr0, dr1) };
                out.extend_from_slice(&[
                    geo.center.x + s * r,
                    geo.center.y - co * r,
                    col[0],
                    col[1],
                    col[2],
                    col[3],
                    da0,
                    da1,
                    dr0,
                    dr1,
                    a.hover,
                    flags,
                    r,
                    seed,
                ]);
            }
            if i > 0 {
                let v = base + (i as u32) * 2;
                indices.extend_from_slice(&[v - 2, v - 1, v, v - 1, v + 1, v]);
            }
        }
    }
}
