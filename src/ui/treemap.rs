//! Treemap view (SpaceSniffer-style nested rectangles), an alternative to the
//! sunburst. Same keys as the sunburst (node id, or group key for "smaller
//! items"), so zooming morphs the clicked tile into the whole chart.

use super::app::{App, CtxMenu, Target};
use super::sunburst::{group_key, view_children, SegKind, View, FREE_KEY};
use super::theme::{lerp_color, lighten, with_alpha, Theme};
use super::view::node_label;
use super::widgets::{self, bold, cr, font};
use crate::i18n::{tr, trf};
use crate::sound::Sfx;
use crate::tree::{fmt_count, fmt_size, Kind, Tree, NONE};
use eframe::egui::{self, Align2, Color32, Id, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use std::collections::HashMap;
use std::time::Instant;

/// Tiles smaller than this (in square points) are folded into "smaller items".
const MIN_AREA: f32 = 90.0;
/// A directory shows its own content when its tile is at least this big.
const NEST_W: f32 = 56.0;
const NEST_H: f32 = 44.0;
const HEADER: f32 = 17.0;
const PAD: f32 = 3.0;
const MAX_TILES: usize = 9000;

#[derive(Clone, Debug)]
pub struct Tile {
    pub key: u64,
    pub node: u32,
    pub kind: SegKind,
    pub rect: Rect,
    pub depth: u8,
    pub color: Color32,
    pub size: u64,
    /// Has visible children drawn inside it (then it shows a header strip).
    pub nested: bool,
}

/// Squarified treemap (Bruls, Huizing, van Wijk). `areas` must be sorted
/// descending and sum to `rect`'s area; returns one rect per area.
fn squarify(areas: &[f32], rect: Rect) -> Vec<Rect> {
    let mut out = Vec::with_capacity(areas.len());
    let mut r = rect;
    let mut i = 0;
    while i < areas.len() {
        let short = r.width().min(r.height()).max(1e-3);
        let worst = |row: &[f32]| -> f32 {
            let s: f32 = row.iter().sum();
            let (mx, mn) = row.iter().fold((0f32, f32::MAX), |(a, b), v| (a.max(*v), b.min(*v)));
            ((short * short * mx) / (s * s)).max((s * s) / (short * short * mn.max(1e-6)))
        };
        let mut j = i + 1;
        while j < areas.len() && worst(&areas[i..j + 1]) <= worst(&areas[i..j]) {
            j += 1;
        }
        let row = &areas[i..j];
        let s: f32 = row.iter().sum();
        if r.width() >= r.height() {
            // lay the row out as a column on the left
            let w = (s / r.height().max(1e-3)).min(r.width());
            let mut y = r.top();
            for a in row {
                let h = a / w.max(1e-3);
                out.push(Rect::from_min_size(Pos2::new(r.left(), y), Vec2::new(w, h)));
                y += h;
            }
            r = Rect::from_min_max(Pos2::new(r.left() + w, r.top()), r.max);
        } else {
            let h = (s / r.width().max(1e-3)).min(r.height());
            let mut x = r.left();
            for a in row {
                let w = a / h.max(1e-3);
                out.push(Rect::from_min_size(Pos2::new(x, r.top()), Vec2::new(w, h)));
                x += w;
            }
            r = Rect::from_min_max(Pos2::new(r.left(), r.top() + h), r.max);
        }
        i = j;
    }
    out
}

fn tile_color(theme: &Theme, kind: Kind, mid: f32, depth: u8) -> Color32 {
    match kind {
        Kind::Dir => theme.segment_color(mid, depth as usize, false),
        Kind::File => theme.segment_color(mid, depth as usize, true),
        Kind::SmallFiles => theme.aggregate_color(),
        Kind::Hidden => lerp_color(theme.neutral, theme.bg_bottom, 0.35),
        Kind::Mount | Kind::Link => theme.text_faint,
    }
}

/// Lays out the view inside `area`.
pub fn layout(tree: &Tree, view: View, area: Rect, max_depth: usize, theme: &Theme, free: u64) -> Vec<Tile> {
    let mut out = Vec::with_capacity(1024);
    if !tree.is_alive(view.node) {
        return out;
    }
    let (kids, total) = view_children(tree, view);
    let root_kind = if view.skip > 0 {
        SegKind::Group { parent: view.node, skip: view.skip, count: kids.len() }
    } else {
        SegKind::Node(tree.get(view.node).kind)
    };
    out.push(Tile { key: view.key(), node: view.node, kind: root_kind, rect: area, depth: 0, color: theme.surface, size: total, nested: true });
    if total == 0 {
        return out;
    }
    // free space takes its share of the area, on the right/bottom
    let mut content = area;
    if free > 0 {
        let frac = free as f32 / (total + free) as f32;
        let (c, f) = if area.width() >= area.height() {
            let w = area.width() * (1.0 - frac);
            (Rect::from_min_size(area.min, Vec2::new(w, area.height())), Rect::from_min_max(Pos2::new(area.left() + w, area.top()), area.max))
        } else {
            let h = area.height() * (1.0 - frac);
            (Rect::from_min_size(area.min, Vec2::new(area.width(), h)), Rect::from_min_max(Pos2::new(area.left(), area.top() + h), area.max))
        };
        content = c;
        out.push(Tile {
            key: FREE_KEY,
            node: view.node,
            kind: SegKind::Free,
            rect: f,
            depth: 1,
            color: lerp_color(theme.bg_top, theme.text_faint, 0.32),
            size: free,
            nested: false,
        });
    }
    // (children, skip offset, parent size, rect, colour range, depth)
    let mut stack: Vec<(Vec<u32>, usize, u64, Rect, (f32, f32), u8)> = vec![(kids, view.skip, total, content, (0.0, 1.0), 1)];
    while let Some((items, base_skip, parent_size, rect, (h0, h1), depth)) = stack.pop() {
        if out.len() > MAX_TILES || rect.width() < 2.0 || rect.height() < 2.0 {
            continue;
        }
        let area_total = rect.width() * rect.height();
        let parent = items.first().map(|c| tree.get(*c).parent).unwrap_or(NONE);
        // decide which items get their own tile; the tail becomes one group
        let mut areas: Vec<f32> = Vec::new();
        let mut shown = 0;
        let mut rest = 0u64;
        for (i, &c) in items.iter().enumerate() {
            let a = area_total * (tree.get(c).size as f64 / parent_size as f64) as f32;
            if a < MIN_AREA && i + 1 < items.len() {
                rest = items[i..].iter().map(|k| tree.get(*k).size).sum();
                break;
            }
            if a < MIN_AREA * 0.25 {
                break;
            }
            areas.push(a);
            shown = i + 1;
        }
        let group_area = area_total * (rest as f64 / parent_size as f64) as f32;
        if rest > 0 && group_area >= MIN_AREA * 0.5 {
            areas.push(group_area);
        }
        // squarify needs the areas to fill the rect exactly
        let sum: f32 = areas.iter().sum();
        if sum <= 0.0 {
            continue;
        }
        let scale = area_total / sum;
        let scaled: Vec<f32> = areas.iter().map(|a| a * scale).collect();
        let rects = squarify(&scaled, rect);
        let mut acc = 0.0f32;
        for (k, r) in rects.iter().enumerate() {
            let frac = scaled[k] / area_total;
            let (c0, c1) = (h0 + (h1 - h0) * acc, h0 + (h1 - h0) * (acc + frac));
            acc += frac;
            let mid = (c0 + c1) / 2.0;
            if k >= shown {
                let skip = base_skip + shown;
                out.push(Tile {
                    key: group_key(parent, skip),
                    node: parent,
                    kind: SegKind::Group { parent, skip, count: items.len() - shown },
                    rect: *r,
                    depth,
                    color: theme.aggregate_color(),
                    size: rest,
                    nested: false,
                });
                continue;
            }
            let c = items[k];
            let n = tree.get(c);
            let nest = n.kind == Kind::Dir && (depth as usize) < max_depth && r.width() >= NEST_W && r.height() >= NEST_H && n.first_child != NONE;
            out.push(Tile {
                key: c as u64,
                node: c,
                kind: SegKind::Node(n.kind),
                rect: *r,
                depth,
                color: tile_color(theme, n.kind, mid, depth),
                size: n.size,
                nested: nest,
            });
            if nest {
                let inner = Rect::from_min_max(Pos2::new(r.left() + PAD, r.top() + HEADER), Pos2::new(r.right() - PAD, r.bottom() - PAD));
                stack.push((tree.sorted_children(c), 0, n.size.max(1), inner, (c0, c1), depth + 1));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Animation: each tile's rect and colour glide towards the new layout.

#[derive(Clone, Debug)]
pub struct TileAnim {
    pub tile: Tile,
    rect: Rect,
    color: Color32,
    alpha: f32,
    hover: f32,
    alive: bool,
}

#[derive(Default)]
pub struct TreemapState {
    pub anims: HashMap<u64, TileAnim>,
    order: Vec<u64>,
    pub sig: Option<(u64, View, usize, [i32; 4], String)>,
    pub last_layout: Option<Instant>,
    pub settled: bool,
}

fn lerp_rect(a: Rect, b: Rect, k: f32) -> Rect {
    Rect::from_min_max(a.min + (b.min - a.min) * k, a.max + (b.max - a.max) * k)
}

impl TreemapState {
    fn set_target(&mut self, tiles: Vec<Tile>) {
        for a in self.anims.values_mut() {
            a.alive = false;
        }
        self.order.clear();
        for t in tiles {
            self.order.push(t.key);
            match self.anims.get_mut(&t.key) {
                Some(a) => {
                    a.alive = true;
                    a.tile = t;
                }
                None => {
                    // new tiles grow out of their centre
                    let start = Rect::from_center_size(t.rect.center(), t.rect.size() * 0.6);
                    self.anims.insert(t.key, TileAnim { rect: start, color: t.color, alpha: 0.0, hover: 0.0, alive: true, tile: t });
                }
            }
        }
        self.settled = false;
    }

    fn step(&mut self, dt: f32, hovered: Option<u64>) {
        let k = 1.0 - (-dt * 12.0).exp();
        let kh = 1.0 - (-dt * 18.0).exp();
        let mut moving = false;
        self.anims.retain(|_, a| a.alive || a.alpha > 0.01);
        for a in self.anims.values_mut() {
            let ht = if Some(a.tile.key) == hovered { 1.0 } else { 0.0 };
            a.hover += (ht - a.hover) * kh;
            if a.alive {
                a.rect = lerp_rect(a.rect, a.tile.rect, k);
                a.color = lerp_color(a.color, a.tile.color, k);
                a.alpha += (1.0 - a.alpha) * k;
                if (a.rect.min - a.tile.rect.min).length() > 0.3 || (a.rect.max - a.tile.rect.max).length() > 0.3 || a.alpha < 0.99 {
                    moving = true;
                }
            } else {
                a.alpha = (a.alpha - a.alpha * k * 1.5 - dt).max(0.0);
                moving = true;
            }
            if (a.hover - ht).abs() > 0.01 {
                moving = true;
            }
        }
        self.settled = !moving;
    }

    /// Centre of a tile's header (or of the tile), for the demo director.
    pub fn center_of(&self, key: u64) -> Option<Pos2> {
        self.anims.get(&key).filter(|a| a.alive).map(|a| if a.tile.nested { Pos2::new(a.rect.center().x, a.rect.top() + HEADER / 2.0) } else { a.rect.center() })
    }

    /// Current colour of a tile (used by the side list).
    pub fn color_of(&self, key: u64) -> Option<Color32> {
        self.anims.get(&key).filter(|a| a.alive).map(|a| a.tile.color)
    }

    fn draw_list(&self) -> Vec<&TileAnim> {
        let mut v: Vec<&TileAnim> = self.anims.values().filter(|a| !a.alive && a.alpha > 0.01).collect();
        v.extend(self.order.iter().filter_map(|k| self.anims.get(k)));
        v
    }

    /// Deepest tile under `p`.
    fn hit(&self, p: Pos2) -> Option<&TileAnim> {
        self.order.iter().rev().filter_map(|k| self.anims.get(k)).find(|a| a.alpha > 0.5 && a.tile.depth > 0 && a.rect.contains(p))
    }
}

impl App {
    pub fn treemap_ui(&mut self, ui: &mut Ui, chart: Rect, panel: Rect, si: usize) {
        let theme = self.theme.clone();
        let depth = self.settings.rings.clamp(3, 9);
        let area = chart.shrink2(Vec2::new(12.0, 10.0));
        let modal_open = self.modal.is_some() || self.ctx_menu.is_some();
        let dragging_node = self.drag.as_ref().filter(|d| d.session == si).map(|d| d.node);

        // free space tile only at the root of a whole-disk scan
        {
            let s = &mut self.sessions[si];
            if let Target::Volume(v) = &s.target {
                if s.free_at.elapsed().as_secs_f32() > 2.0 {
                    s.free_space = if crate::scan::demo::enabled() {
                        v.available
                    } else {
                        crate::scan::platform::volume_space(std::path::Path::new(&v.mount)).map(|x| x.1).unwrap_or(v.available)
                    };
                    s.free_at = Instant::now();
                }
            }
        }

        // layout (throttled while scanning)
        {
            let s = &mut self.sessions[si];
            let t = s.tree.read().unwrap();
            let free = if s.view == View::node(t.root) && matches!(s.target, Target::Volume(_)) { s.free_space } else { 0 };
            let r = [area.left() as i32, area.top() as i32, area.width() as i32, area.height() as i32];
            let sig = (t.version ^ (free >> 20).rotate_left(40), s.view, depth, r, theme.name.to_string());
            let changed_view = s.treemap.sig.as_ref().is_none_or(|old| old.1 != sig.1 || old.3 != sig.3 || old.4 != sig.4 || old.2 != sig.2);
            let interval = if s.is_scanning() { 160 } else { 30 };
            let due = s.treemap.last_layout.is_none_or(|l| l.elapsed().as_millis() > interval);
            if s.treemap.sig.as_ref() != Some(&sig) && (changed_view || due) {
                let tiles = layout(&t, s.view, area, depth, &theme, free);
                drop(t);
                s.treemap.set_target(tiles);
                s.treemap.sig = Some(sig);
                s.treemap.last_layout = Some(Instant::now());
            }
        }

        // interaction
        let resp = ui.interact(chart, Id::new(("treemap", si)), Sense::click_and_drag());
        let pointer = ui.ctx().input(|i| i.pointer.latest_pos());
        let hit = if !modal_open && self.drag.is_none() {
            pointer.filter(|p| area.contains(*p)).and_then(|p| self.sessions[si].treemap.hit(p).map(|a| a.tile.clone()))
        } else {
            None
        };
        {
            let s = &mut self.sessions[si];
            s.hovered_key = hit.as_ref().map(|t| t.key).or(s.list_hover.map(|n| n as u64));
            let in_panel = pointer.is_some_and(|p| panel.contains(p));
            if let Some(t) = &hit {
                s.panel_view = match t.kind {
                    SegKind::Node(Kind::Dir) => View::node(t.node),
                    SegKind::Group { parent, skip, .. } => View { node: parent, skip },
                    SegKind::Free => s.view,
                    _ => {
                        let tr_ = s.tree.read().unwrap();
                        if tr_.is_alive(t.node) { View::node(tr_.get(t.node).parent) } else { s.view }
                    }
                };
                s.selected = if t.kind != SegKind::Free { Some(t.node) } else { None };
            } else if !in_panel && self.drag.is_none() {
                s.panel_view = s.view;
                s.selected = None;
            }
        }
        if hit.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            if let Some(t) = hit.clone() {
                match t.kind {
                    SegKind::Group { parent, skip, .. } => {
                        self.sessions[si].navigate(View { node: parent, skip });
                        self.sfx(Sfx::Blip);
                    }
                    SegKind::Node(Kind::Dir) => {
                        self.sessions[si].navigate(View::node(t.node));
                        self.sfx(Sfx::Blip);
                    }
                    SegKind::Node(Kind::SmallFiles) => self.expand(si, t.node),
                    _ => {}
                }
            } else if pointer.is_some_and(|p| area.contains(p)) && self.sessions[si].go_up() {
                self.sfx(Sfx::BlipDown);
            }
        }
        if resp.double_clicked() {
            if let Some(t) = &hit {
                if matches!(t.kind, SegKind::Node(Kind::File)) && self.sessions[si].source().is_local() {
                    let path = self.sessions[si].tree.read().unwrap().path(t.node);
                    super::app::os_open(&path, false);
                }
            }
        }
        if resp.secondary_clicked() {
            if let (Some(t), Some(p)) = (&hit, pointer) {
                if matches!(t.kind, SegKind::Node(_)) {
                    self.ctx_menu = Some(CtxMenu { session: si, node: t.node, pos: p, opened: Instant::now() });
                }
            }
        }
        if resp.drag_started() && self.drag.is_none() {
            if let Some(o) = ui.ctx().input(|i| i.pointer.press_origin()) {
                let t = self.sessions[si].treemap.hit(o).map(|a| (a.tile.clone(), a.color));
                if let Some((t, color)) = t {
                    if let SegKind::Node(k) = t.kind {
                        if !matches!(k, Kind::Hidden | Kind::Mount) {
                            self.begin_drag(si, t.node, color, o);
                        }
                    }
                }
            }
        }
        if !modal_open && self.drag.is_none() && ui.ctx().input(|i| i.key_pressed(egui::Key::Delete)) {
            if let Some(t) = &hit {
                if self.sessions[si].collect(t.node) {
                    self.sfx(Sfx::Plop);
                    self.bin_bounce = 1.0;
                }
            }
        }

        // animate + draw
        let dt = self.dt;
        let s = &mut self.sessions[si];
        let hk = s.hovered_key;
        s.treemap.step(dt, hk);
        let t = s.tree.read().unwrap();
        let root = t.root;
        let collector = s.collector.clone();
        let p = ui.painter().with_clip_rect(area.expand(2.0));
        for a in s.treemap.draw_list() {
            if a.tile.depth == 0 {
                continue;
            }
            let r = a.rect.shrink(0.5);
            if r.width() < 1.0 || r.height() < 1.0 {
                continue;
            }
            let collected = matches!(a.tile.kind, SegKind::Node(_))
                && a.tile.node != root
                && t.is_alive(a.tile.node)
                && collector.iter().any(|c| *c == a.tile.node || t.is_ancestor(*c, a.tile.node));
            let mut fill = a.color;
            if a.tile.nested {
                // frame of a folder whose content is drawn inside: a deeper shade
                fill = lerp_color(fill, theme.bg_bottom, 0.55);
            }
            if collected {
                let (h, _, l) = super::theme::to_hsl(fill);
                fill = super::theme::from_hsl(h, 0.08, l * 0.8);
            }
            if Some(a.tile.node) == dragging_node && matches!(a.tile.kind, SegKind::Node(_)) {
                fill = lerp_color(fill, theme.bg_bottom, 0.5);
            }
            fill = lerp_color(fill, Color32::WHITE, 0.14 * a.hover);
            let radius = if r.width() > 12.0 && r.height() > 12.0 { 3.0 } else { 1.0 };
            p.rect_filled(r, cr(radius), with_alpha(fill, a.alpha));
            if a.hover > 0.01 {
                p.rect_stroke(r, cr(radius), Stroke::new(1.5, with_alpha(Color32::WHITE, 0.6 * a.hover * a.alpha)), egui::StrokeKind::Inside);
            }
            if collected && r.width() > 8.0 && r.height() > 8.0 {
                // diagonal hatch for items waiting in the collector
                let hp = p.with_clip_rect(r.intersect(area));
                let mut x = r.left() - r.height();
                while x < r.right() {
                    hp.line_segment([Pos2::new(x, r.bottom()), Pos2::new(x + r.height(), r.top())], Stroke::new(1.0, with_alpha(Color32::WHITE, 0.12 * a.alpha)));
                    x += 7.0;
                }
            }
            // labels
            let label = match a.tile.kind {
                SegKind::Group { count, .. } => trf("{count} smaller items", &[("count", &fmt_count(count as u64))]),
                SegKind::Free => tr("Free space").to_string(),
                _ if t.is_alive(a.tile.node) => node_label(&t, a.tile.node),
                _ => String::new(),
            };
            let text_col = if theme.dark || a.tile.nested { theme.text } else { Color32::from_rgb(0x1a, 0x12, 0x27) };
            let tc = with_alpha(if a.tile.nested { theme.text } else { contrast_text(fill, text_col) }, a.alpha);
            let lp = p.with_clip_rect(r.intersect(area));
            if a.tile.nested {
                if r.width() > 40.0 {
                    let size = fmt_size(a.tile.size);
                    let sw = lp.layout_no_wrap(size.clone(), font(10.5), tc).size().x;
                    let name = widgets::truncate(&lp, &label, &bold(11.0), r.width() - sw - 18.0);
                    lp.text(Pos2::new(r.left() + 5.0, r.top() + HEADER / 2.0), Align2::LEFT_CENTER, name, bold(11.0), tc);
                    lp.text(Pos2::new(r.right() - 5.0, r.top() + HEADER / 2.0), Align2::RIGHT_CENTER, size, font(10.5), with_alpha(tc, 0.75 * a.alpha));
                }
            } else if r.width() > 46.0 && r.height() > 22.0 {
                let name = widgets::truncate(&lp, &label, &bold(11.5), r.width() - 10.0);
                lp.text(Pos2::new(r.left() + 5.0, r.top() + 5.0), Align2::LEFT_TOP, name, bold(11.5), tc);
                if r.height() > 38.0 {
                    lp.text(Pos2::new(r.left() + 5.0, r.top() + 21.0), Align2::LEFT_TOP, fmt_size(a.tile.size), font(10.5), with_alpha(tc, 0.75 * a.alpha));
                }
            }
        }
        let empty = s.treemap.anims.len() <= 1;
        let scanning = s.is_scanning();
        drop(t);
        if empty {
            let msg = if scanning { tr("Scanning…") } else { tr("Empty") };
            ui.painter().text(area.center(), Align2::CENTER_CENTER, msg, font(13.0), theme.text_faint);
        }
        let _ = lighten;
    }
}

/// Dark or light text depending on the tile colour.
fn contrast_text(bg: Color32, dark_default: Color32) -> Color32 {
    let l = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    if l > 150.0 {
        Color32::from_rgb(0x1a, 0x12, 0x27)
    } else if dark_default == Color32::from_rgb(0x1a, 0x12, 0x27) {
        Color32::WHITE
    } else {
        dark_default
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squarify_fills_the_rect() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0));
        let areas = [96000.0, 72000.0, 48000.0, 12000.0, 12000.0];
        let rects = squarify(&areas, rect);
        assert_eq!(rects.len(), areas.len());
        let total: f32 = rects.iter().map(|r| r.width() * r.height()).sum();
        assert!((total - 240_000.0).abs() < 1.0);
        for (r, a) in rects.iter().zip(areas) {
            assert!((r.width() * r.height() - a).abs() < 1.0);
            assert!(rect.expand(0.01).contains_rect(*r));
        }
    }
}
