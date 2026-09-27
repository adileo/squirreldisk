//! The chart screen: breadcrumb, sunburst, side list and the collector.

use super::app::{App, CtxMenu, DeleteItem, Modal, Screen, Target};
use super::fx::Drag;
use super::shader;
use super::sunburst::{self, Geometry, SegKind, SegStyle, View};
use super::theme::{lerp_color, lighten, with_alpha, Theme};
use super::widgets::{self, bold, cr, display, font, BtnStyle, Icon};
use crate::safety::{Os, Rules};
use crate::scan::local::expand_small;
use crate::sound::Sfx;
use crate::tree::{fmt_count, fmt_size, Kind, Tree, F_EXPANDABLE, NONE};
use eframe::egui::{self, Align2, Color32, Id, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};
use std::f32::consts::TAU;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

pub fn node_label(t: &Tree, id: u32) -> String {
    let n = t.get(id);
    match n.kind {
        Kind::SmallFiles => format!("{} smaller files", fmt_count(n.files as u64)),
        Kind::Mount => format!("{} (other volume)", n.name),
        _ => n.name.to_string(),
    }
}

fn view_label(t: &Tree, v: View) -> String {
    if v.skip > 0 {
        let n = t.child_count(v.node).saturating_sub(v.skip);
        format!("{} smaller items", fmt_count(n as u64))
    } else {
        node_label(t, v.node)
    }
}

impl App {
    pub fn session_ui(&mut self, ui: &mut Ui, screen: Rect, si: usize) {
        let theme = self.theme.clone();
        // keep the view alive if things were deleted underneath
        {
            let s = &mut self.sessions[si];
            let t = s.tree.read().unwrap();
            let mut v = s.view.node;
            while v != NONE && !t.is_alive(v) {
                v = t.get(v).parent;
            }
            let fix = v != s.view.node;
            let pv_dead = !t.is_alive(s.panel_view.node);
            drop(t);
            if fix {
                s.view = View::node(if v == NONE { 0 } else { v });
            }
            if fix || pv_dead {
                s.panel_view = s.view;
            }
        }

        // --- top bar
        // The bar is centred on the traffic lights (macOS) so everything sits on one line.
        let bar_h = self.chrome.0 * 2.0;
        let bar = Rect::from_min_size(screen.min, Vec2::new(screen.width(), bar_h));
        self.window_drag_area(ui, Rect::from_min_size(screen.min, Vec2::new(screen.width(), bar_h + 4.0)));
        self.top_bar(ui, bar, si);

        let body = Rect::from_min_max(Pos2::new(screen.left(), bar.bottom() + 4.0), screen.max);
        let panel_w = (body.width() * 0.36).clamp(300.0, 440.0);
        let banner = Rect::from_min_max(Pos2::new(body.right() - panel_w - 16.0, body.bottom() - 16.0 - 52.0), Pos2::new(body.right() - 16.0, body.bottom() - 16.0));
        let panel = Rect::from_min_max(Pos2::new(body.right() - panel_w - 16.0, body.top() + 6.0), Pos2::new(body.right() - 16.0, banner.top() - 10.0));
        let chart = Rect::from_min_max(Pos2::new(body.left() + 8.0, body.top()), Pos2::new(panel.left() - 8.0, body.bottom() - 76.0));
        let collector = Rect::from_min_size(Pos2::new(body.left() + 20.0, body.bottom() - 70.0), Vec2::new((chart.width() - 24.0).min(460.0), 54.0));

        self.chart_ui(ui, chart, panel, si);
        self.panel_ui(ui, panel, si);
        self.sponsor_banner(ui, banner, "panel");
        self.collector_ui(ui, collector, si);

        // Linux: follow the view with inotify watches
        let s = &mut self.sessions[si];
        if s.watch.is_some() && s.watch_focus != Some(s.view) {
            s.watch_focus = Some(s.view);
            let dirs: Vec<std::path::PathBuf> = {
                let t = s.tree.read().unwrap();
                let mut v = vec![t.path_buf(s.view.node)];
                v.extend(t.children(s.view.node).filter(|c| t.get(*c).kind == Kind::Dir).map(|c| t.path_buf(c)));
                v
            };
            if let Some(w) = s.watch.as_mut() {
                w.focus(dirs);
            }
        }
        let _ = theme;
    }

    fn top_bar(&mut self, ui: &mut Ui, bar: Rect, si: usize) {
        let theme = self.theme.clone();
        let mut x = bar.left() + if cfg!(target_os = "macos") { self.chrome.1 + 18.0 } else { 14.0 };
        let cy = bar.center().y;
        let nav = 24.0;
        let can_fwd = !self.sessions[si].fwd.is_empty();
        if widgets::icon_button(ui, Rect::from_center_size(Pos2::new(x + nav / 2.0, cy), Vec2::splat(nav)), Id::new("nav-back"), Icon::Back, &theme, true).clicked() {
            if !self.sessions[si].go_back() {
                self.screen = Screen::Home;
            }
            self.sfx(Sfx::BlipDown);
        }
        x += nav + 2.0;
        if widgets::icon_button(ui, Rect::from_center_size(Pos2::new(x + nav / 2.0, cy), Vec2::splat(nav)), Id::new("nav-fwd"), Icon::Forward, &theme, can_fwd).clicked() {
            self.sessions[si].go_forward();
            self.sfx(Sfx::Blip);
        }
        x += nav + 12.0;

        // breadcrumb: ghost links
        let crumbs: Vec<(String, Option<View>)> = {
            let s = &self.sessions[si];
            let t = s.tree.read().unwrap();
            let mut v: Vec<(String, Option<View>)> = vec![("Disks".into(), None)];
            for id in t.ancestry(s.view.node) {
                let name = if id == t.root { s.title.clone() } else { t.get(id).name.to_string() };
                v.push((name, Some(View::node(id))));
            }
            if s.view.skip > 0 {
                v.push((view_label(&t, s.view), Some(s.view)));
            }
            v
        };
        let f_link = font(13.0);
        let f_last = bold(13.0);
        let right_limit = bar.right() - 90.0;
        let mut shown = crumbs.clone();
        let widths = |v: &Vec<(String, Option<View>)>, p: &egui::Painter| -> f32 {
            v.iter().map(|(n, _)| p.layout_no_wrap(n.clone(), bold(13.0), Color32::WHITE).size().x.min(200.0) + 30.0).sum()
        };
        while shown.len() > 3 && x + widths(&shown, ui.painter()) > right_limit {
            if shown[1].0 == "\u{2026}" {
                shown.remove(2);
            } else {
                shown[1] = ("\u{2026}".into(), shown[1].1);
            }
        }
        let n = shown.len();
        for (k, (name, target)) in shown.into_iter().enumerate() {
            let last = k + 1 == n;
            let fnt = if last { f_last.clone() } else { f_link.clone() };
            let label = widgets::truncate(ui.painter(), &name, &fnt, 200.0);
            let tw = ui.painter().layout_no_wrap(label.clone(), fnt.clone(), Color32::WHITE).size().x;
            let r = Rect::from_min_size(Pos2::new(x, cy - 11.0), Vec2::new(tw + 12.0, 22.0));
            let id = Id::new(("crumb", k));
            let resp = ui.interact(r, id, if last { Sense::hover() } else { Sense::click() });
            let h = ui.ctx().animate_bool_with_time(id.with("h"), resp.hovered() && !last, 0.1);
            let p = ui.painter();
            if h > 0.0 {
                p.rect_filled(r, cr(5.0), with_alpha(theme.surface_hi, 0.7 * h));
            }
            let col = if last { theme.text } else { lerp_color(theme.text_dim, theme.text, h) };
            p.text(Pos2::new(r.left() + 6.0, cy), Align2::LEFT_CENTER, &label, fnt, col);
            if resp.hovered() && !last {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                match target {
                    None => self.screen = Screen::Home,
                    Some(v) => self.sessions[si].navigate(v),
                }
                self.sfx(Sfx::BlipDown);
            }
            x += r.width();
            if !last {
                ui.painter().text(Pos2::new(x + 4.0, cy), Align2::CENTER_CENTER, "/", font(13.0), theme.text_faint);
                x += 8.0;
            }
        }

        // right side
        let gear = Rect::from_center_size(Pos2::new(bar.right() - 24.0, cy), Vec2::splat(nav));
        self.mark("settings", gear);
        if widgets::icon_button(ui, gear, Id::new("settings2"), Icon::Gear, &theme, true).clicked() {
            self.open_modal(Modal::Settings);
        }
        let scanning = self.sessions[si].is_scanning();
        let rs = Rect::from_center_size(Pos2::new(gear.left() - 18.0, cy), Vec2::splat(nav));
        if widgets::icon_button(ui, rs, Id::new("rescan"), if scanning { Icon::Close } else { Icon::Refresh }, &theme, true).clicked() {
            if scanning {
                self.sessions[si].progress.cancel.store(true, Ordering::Relaxed);
            } else {
                let t = self.sessions[si].target.clone();
                self.start_session(t);
            }
        }
    }

    fn chart_ui(&mut self, ui: &mut Ui, chart: Rect, panel: Rect, si: usize) {
        let theme = self.theme.clone();
        let rings = self.settings.rings.clamp(3, 9);
        let radius = (chart.width().min(chart.height()) / 2.0 - 18.0).max(60.0);
        let geo = Geometry::new(chart.center(), radius, rings);
        self.sessions[si].geo = Some(geo);
        let dragging_node = self.drag.as_ref().filter(|d| d.session == si).map(|d| d.node);
        let modal_open = self.modal.is_some() || self.ctx_menu.is_some();

        // --- free space slice (root of a whole-disk scan)
        {
            let s = &mut self.sessions[si];
            if let Target::Volume(v) = &s.target {
                if s.free_at.elapsed().as_secs_f32() > 2.0 {
                    s.free_space = if crate::scan::demo::enabled() { v.available } else { crate::scan::platform::volume_space(std::path::Path::new(&v.mount)).map(|x| x.1).unwrap_or(v.available) };
                    s.free_at = Instant::now();
                }
            }
        }
        // --- layout (throttled while scanning)
        {
            let s = &mut self.sessions[si];
            let t = s.tree.read().unwrap();
            let free = if s.view == View::node(t.root) && matches!(s.target, Target::Volume(_)) { s.free_space } else { 0 };
            // include free space (MB resolution) so the slice follows deletions
            let sig = (t.version ^ (free >> 20).rotate_left(40), s.view, rings, radius as i32, theme.name.to_string());
            let view_changed = sig.1 != s.layout_sig.1 || sig.2 != s.layout_sig.2 || sig.4 != s.layout_sig.4;
            let interval = if s.is_scanning() { 160 } else { 30 };
            if sig != s.layout_sig && (view_changed || s.last_layout.elapsed().as_millis() > interval) {
                let segs = sunburst::layout(&t, s.view, &geo, rings, &theme, free);
                drop(t);
                let intro = s.intro && !s.is_scanning();
                s.anim.set_target(&segs, intro || s.intro);
                s.intro = false;
                s.layout_sig = sig;
                s.last_layout = Instant::now();
            }
        }

        // --- interaction
        let resp = ui.interact(chart, Id::new(("chart", si)), Sense::click_and_drag());
        let pointer = ui.ctx().input(|i| i.pointer.latest_pos());
        let hit = if !modal_open && self.drag.is_none() {
            pointer.filter(|p| chart.contains(*p)).and_then(|p| self.sessions[si].anim.hit(&geo, p).cloned())
        } else {
            None
        };
        {
            let s = &mut self.sessions[si];
            s.hovered_key = hit.as_ref().map(|a| a.key).or(s.list_hover.map(|n| n as u64));
            let in_panel = pointer.is_some_and(|p| panel.contains(p));
            if let Some(a) = &hit {
                s.panel_view = match a.kind {
                    SegKind::Node(Kind::Dir) if a.d > 0.5 => View::node(a.node),
                    SegKind::Group { parent, skip, .. } if a.d > 0.5 => View { node: parent, skip },
                    _ if a.d < 0.5 => s.view,
                    SegKind::Free => s.view,
                    _ => {
                        let t = s.tree.read().unwrap();
                        if t.is_alive(a.node) { View::node(t.get(a.node).parent) } else { s.view }
                    }
                };
                s.selected = if a.d > 0.5 && a.kind != SegKind::Free { Some(a.node) } else { None };
            } else if !in_panel && self.drag.is_none() {
                s.panel_view = s.view;
                s.selected = None;
            }
        }
        if hit.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            if let Some(a) = hit.clone() {
                self.chart_click(si, &a);
            }
        }
        if resp.double_clicked() {
            if let Some(a) = &hit {
                if matches!(a.kind, SegKind::Node(Kind::File)) {
                    let path = self.sessions[si].tree.read().unwrap().path(a.node);
                    if self.sessions[si].source().is_local() {
                        super::app::os_open(&path, false);
                    }
                }
            }
        }
        if resp.secondary_clicked() {
            if let (Some(a), Some(p)) = (&hit, pointer) {
                if matches!(a.kind, SegKind::Node(_)) {
                    self.ctx_menu = Some(CtxMenu { session: si, node: a.node, pos: p, opened: Instant::now() });
                }
            }
        }
        if resp.drag_started() && self.drag.is_none() {
            let origin = ui.ctx().input(|i| i.pointer.press_origin());
            if let Some(o) = origin {
                let a = self.sessions[si].anim.hit(&geo, o).cloned();
                if let Some(a) = a {
                    if let SegKind::Node(k) = a.kind {
                        let is_root = self.sessions[si].tree.read().unwrap().root == a.node;
                        if !matches!(k, Kind::Hidden | Kind::Mount) && !is_root {
                            self.begin_drag(si, a.node, a.color, o);
                        }
                    }
                }
            }
        }
        if !modal_open && self.drag.is_none() && ui.ctx().input(|i| i.key_pressed(egui::Key::Delete)) {
            if let Some(a) = &hit {
                if self.sessions[si].collect(a.node) {
                    self.sfx(Sfx::Plop);
                    self.bin_bounce = 1.0;
                }
            }
        }

        // --- animate
        let dt = self.dt;
        {
            let s = &mut self.sessions[si];
            let hk = s.hovered_key;
            s.anim.step(dt, hk);
        }

        // --- background decoration
        {
            let p = ui.painter();
            widgets::glow(p, geo.center, radius * 1.2, with_alpha(theme.accent, if theme.dark { 0.05 } else { 0.03 }));
            p.circle_filled(geo.center, radius + 6.0, with_alpha(theme.bg_bottom, 0.25));
        }

        // --- geometry
        let s = &mut self.sessions[si];
        let t = s.tree.read().unwrap();
        let root = t.root;
        let collector = s.collector.clone();
        let draw = s.anim.draw_list();
        let mut verts = std::mem::take(&mut s.verts);
        let mut idx = std::mem::take(&mut s.idx);
        let scanning = s.is_scanning();
        sunburst::build_vertices(
            &draw,
            &geo,
            |a| {
                let mut flags = 0.0;
                if let SegKind::Node(k) = a.kind {
                    if a.node != root && t.is_alive(a.node) && collector.iter().any(|c| *c == a.node || t.is_ancestor(*c, a.node)) {
                        flags += sunburst::FLAG_COLLECTED;
                    }
                    if Some(a.node) == dragging_node {
                        flags += sunburst::FLAG_DRAGGED;
                    }
                    if matches!(k, Kind::SmallFiles | Kind::Hidden) {
                        flags += sunburst::FLAG_AGGREGATE;
                    }
                } else {
                    flags += sunburst::FLAG_AGGREGATE;
                }
                if scanning {
                    flags += sunburst::FLAG_SCANNING;
                }
                SegStyle { flags, lift: 0.0 }
            },
            &mut verts,
            &mut idx,
        );
        let hovered_anim = s.hovered_key.and_then(|k| s.anim.anims.get(&k)).filter(|a| a.d > 0.5).cloned();
        let center_anim_color = s.anim.anims.get(&s.view.key()).map(|a| a.color);
        let view = s.view;
        let center_label = view_label(&t, view);
        let center_size = sunburst::view_size(&t, view);
        let hovered_label = hovered_anim.as_ref().map(|a| match a.kind {
            SegKind::Group { count, .. } => (format!("{} smaller items", fmt_count(count as u64)), a.size),
            SegKind::Free => ("Free space".to_string(), a.size),
            _ => (if t.is_alive(a.node) { node_label(&t, a.node) } else { String::new() }, a.size),
        });
        drop(t);

        let screen = ui.ctx().content_rect();
        if let Some(gl) = &self.gl {
            let frame = shader::Frame {
                vertices: verts.clone(),
                indices: idx.clone(),
                screen: [screen.width(), screen.height()],
                center: [geo.center.x, geo.center.y],
                time: self.time as f32,
                fx: self.settings.shader_fx,
            };
            ui.painter().with_clip_rect(chart.expand(30.0)).add(shader::callback(gl.clone(), Arc::new(frame), screen));
        } else {
            cpu_mesh(ui.painter(), &verts, &idx);
        }
        let s = &mut self.sessions[si];
        s.verts = verts;
        s.idx = idx;

        // --- center label
        let p = ui.painter();
        let r0 = geo.r0;
        let _ = center_anim_color;
        let (title, size) = hovered_label.unwrap_or((center_label, center_size));
        let size_txt = fmt_size(size);
        let fs = (r0 * 0.42).clamp(14.0, 34.0);
        let g = p.layout_no_wrap(size_txt.clone(), display(fs), theme.text);
        let fs = if g.size().x > r0 * 1.7 { fs * r0 * 1.7 / g.size().x } else { fs };
        p.text(geo.center - Vec2::new(0.0, fs * 0.22), Align2::CENTER_CENTER, size_txt, display(fs), theme.text);
        let name = widgets::truncate(p, &title, &font(12.0), r0 * 1.6);
        p.text(geo.center + Vec2::new(0.0, fs * 0.55), Align2::CENTER_CENTER, name, font(12.0), theme.text_dim);

        // --- scan status
        let s = &self.sessions[si];
        if let Some(err) = s.progress.error.lock().unwrap().clone() {
            if err != "cancelled" && center_size == 0 {
                let r = Rect::from_center_size(geo.center + Vec2::new(0.0, r0 + 50.0), Vec2::new(chart.width().min(460.0), 60.0));
                p.rect_filled(r, cr(14.0), with_alpha(theme.danger, 0.18));
                let msg = widgets::truncate(p, &err, &font(12.5), r.width() - 30.0);
                p.text(r.center() - Vec2::new(0.0, 9.0), Align2::CENTER_CENTER, "Scan failed", bold(13.0), theme.danger);
                p.text(r.center() + Vec2::new(0.0, 10.0), Align2::CENTER_CENTER, msg, font(12.5), theme.text);
            }
        } else if s.is_scanning() {
            let pr = &s.progress;
            let frac = pr.fraction();
            // spinning arc around the center disc
            let a0 = (self.time * 2.2) as f32;
            let pts: Vec<Pos2> = (0..=32).map(|i| geo.point(r0 - 5.0, a0 + i as f32 / 32.0 * 1.4)).collect();
            p.add(Shape::line(pts, Stroke::new(2.5, theme.accent2)));
            if let Some(f) = frac {
                widgets::ring(p, geo.center, r0 + 1.0, 2.0, f, theme.accent, Color32::TRANSPARENT);
            }
            let status = pr.status.lock().unwrap().clone();
            let files = pr.files.load(Ordering::Relaxed);
            let bytes = pr.bytes.load(Ordering::Relaxed);
            let txt = match frac {
                Some(f) => format!("{status} · {:.0}% · {} files · {}", f * 100.0, crate::tree::fmt_count_compact(files), fmt_size(bytes)),
                None => format!("{status} · {} files · {}", crate::tree::fmt_count_compact(files), fmt_size(bytes)),
            };
            let g = p.layout_no_wrap(txt.clone(), bold(12.0), theme.text);
            let pill = Rect::from_center_size(Pos2::new(chart.center().x, chart.top() + 18.0), Vec2::new(g.size().x + 40.0, 30.0));
            widgets::shadow(p, pill, 8.0, 0.6, &theme);
            p.rect_filled(pill, cr(8.0), with_alpha(theme.surface_hi, 0.95));
            p.circle_filled(Pos2::new(pill.left() + 16.0, pill.center().y), 3.5, with_alpha(theme.accent2, 0.6 + 0.4 * (self.time as f32 * 4.0).sin().abs()));
            p.galley(Pos2::new(pill.left() + 26.0, pill.center().y - g.size().y / 2.0), g, theme.text);
            // Only surface a path when a folder is blocking us (network mount, huge dir…).
            if let Some((cur, secs)) = pr.stuck(10.0) {
                let c = widgets::truncate(p, &format!("Still reading {cur} ({secs:.0}s)"), &font(11.0), chart.width() - 60.0);
                p.text(Pos2::new(chart.center().x, pill.bottom() + 12.0), Align2::CENTER_CENTER, c, font(11.0), theme.warn);
            }
        }
    }

    fn chart_click(&mut self, si: usize, a: &sunburst::Anim) {
        let s = &mut self.sessions[si];
        if a.d < 0.5 {
            if s.go_up() {
                self.sfx(Sfx::BlipDown);
            }
            return;
        }
        match a.kind {
            SegKind::Group { parent, skip, .. } => {
                s.navigate(View { node: parent, skip });
                self.sfx(Sfx::Blip);
            }
            SegKind::Node(Kind::Dir) => {
                s.navigate(View::node(a.node));
                self.sfx(Sfx::Blip);
            }
            SegKind::Node(Kind::SmallFiles) => self.expand(si, a.node),
            _ => {}
        }
    }

    fn expand(&mut self, si: usize, node: u32) {
        let s = &self.sessions[si];
        let expandable = {
            let t = s.tree.read().unwrap();
            t.is_alive(node) && t.get(node).flags & F_EXPANDABLE != 0 && t.source.is_local()
        };
        if !expandable {
            self.toasts.push("These files can't be listed individually here".into(), self.theme.text_faint);
            return;
        }
        if s.expanding.swap(true, Ordering::Relaxed) {
            return;
        }
        let (tree, flag) = (s.tree.clone(), s.expanding.clone());
        std::thread::spawn(move || {
            expand_small(&tree, node);
            flag.store(false, Ordering::Relaxed);
        });
        self.sfx(Sfx::Blip);
    }

    pub fn begin_drag(&mut self, si: usize, node: u32, color: Color32, at: Pos2) {
        let (label, size) = {
            let t = self.sessions[si].tree.read().unwrap();
            if !t.is_alive(node) {
                return;
            }
            (node_label(&t, node), t.get(node).size)
        };
        self.drag = Some(Drag { session: si, node, label, size, color, pos: at, vel: Vec2::ZERO, over_bin: false, started: Instant::now() });
        self.sfx(Sfx::Pick);
    }

    fn panel_ui(&mut self, ui: &mut Ui, panel: Rect, si: usize) {
        let theme = self.theme.clone();
        {
            let p = ui.painter();
            widgets::shadow(p, panel, 18.0, 0.5, &theme);
            p.rect_filled(panel, cr(12.0), with_alpha(theme.surface, if theme.dark { 0.72 } else { 0.9 }));
            p.rect_stroke(panel, cr(12.0), Stroke::new(1.0, with_alpha(theme.stroke, 0.6)), egui::StrokeKind::Inside);
        }
        let s = &self.sessions[si];
        let pv = s.panel_view;
        let selected = s.selected;
        let list_hover_before = s.list_hover;
        let t = s.tree.read().unwrap();
        if !t.is_alive(pv.node) {
            return;
        }
        let mut kids = t.sorted_children(pv.node);
        if pv.skip > 0 {
            kids = kids.split_off(pv.skip.min(kids.len()));
        }
        let total = sunburst::view_size(&t, pv).max(1);
        let title = if pv.node == t.root && pv.skip == 0 { s.title.clone() } else { view_label(&t, pv) };
        let files = t.get(pv.node).files;
        let inner = panel.shrink2(Vec2::new(18.0, 16.0));

        // header
        let p = ui.painter().clone();
        let size_txt = fmt_size(total);
        let sg = p.layout_no_wrap(size_txt.clone(), display(20.0), theme.text);
        let tw = inner.width() - sg.size().x - 14.0;
        let tt = widgets::truncate(&p, &title, &display(18.0), tw);
        p.text(inner.left_top() + Vec2::new(0.0, 2.0), Align2::LEFT_TOP, tt, display(20.0), theme.text);
        p.galley(Pos2::new(inner.right() - sg.size().x, inner.top() + 1.0), sg, theme.text);
        // live counts change fast: keep them compact until the scan is done
        let count = |n: u64| if s.is_scanning() { crate::tree::fmt_count_compact(n) } else { fmt_count(n) };
        let sub = format!("{} files · {} items", count(files as u64), count(kids.len() as u64));
        p.text(inner.left_top() + Vec2::new(0.0, 32.0), Align2::LEFT_TOP, sub, font(12.0), theme.text_faint);

        // rows
        let row_h = 30.0;
        let top = inner.top() + 58.0;
        let footer_h = if pv.node == t.root && matches!(s.target, Target::Volume(_)) { 64.0 } else { 26.0 };
        let avail = inner.bottom() - footer_h - top;
        let max_rows = ((avail / row_h).floor() as usize).max(1);
        let overflow = kids.len() > max_rows;
        let show = if overflow { max_rows - 1 } else { kids.len() };
        struct Row {
            node: u32,
            label: String,
            size: u64,
            color: Color32,
            kind: Kind,
            denied: bool,
            collected: bool,
        }
        let rows: Vec<Row> = kids[..show]
            .iter()
            .map(|&c| {
                let n = t.get(c);
                let color = s.anim.anims.get(&(c as u64)).map(|a| a.color).unwrap_or(match n.kind {
                    Kind::Dir | Kind::File => theme.wheel_at(0.0),
                    _ => theme.neutral,
                });
                Row { node: c, label: node_label(&t, c), size: n.size, color, kind: n.kind, denied: n.denied(), collected: s.is_collected(&t, c) }
            })
            .collect();
        let more = if overflow {
            let rest: u64 = kids[show..].iter().map(|c| t.get(*c).size).sum();
            Some((kids.len() - show, rest, View { node: pv.node, skip: pv.skip + show }))
        } else {
            None
        };
        let hovered_path = selected.or(list_hover_before).filter(|n| t.is_alive(*n)).map(|n| t.path(n));
        let is_root_volume = pv.node == t.root && pv.skip == 0;
        drop(t);

        let mut new_hover = None;
        let mut action: Option<(u32, Kind, u8)> = None; // 0 click, 1 double, 2 context
        for (k, r) in rows.iter().enumerate() {
            let rr = Rect::from_min_size(Pos2::new(inner.left() - 8.0, top + k as f32 * row_h), Vec2::new(inner.width() + 16.0, row_h - 2.0));
            let id = Id::new(("row", si, r.node));
            let resp = ui.interact(rr, id, Sense::click_and_drag());
            let hl = resp.hovered() || selected == Some(r.node);
            let h = ui.ctx().animate_bool_with_time(id.with("h"), hl, 0.1);
            if resp.hovered() {
                new_hover = Some(r.node);
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let p = ui.painter();
            if h > 0.0 {
                p.rect_filled(rr, cr(8.0), with_alpha(theme.surface_hi, h));
            }
            let dot = Pos2::new(rr.left() + 16.0, rr.center().y - 1.0);
            match r.kind {
                Kind::Dir => {
                    p.circle_filled(dot, 5.0 + h, r.color);
                }
                Kind::File => {
                    p.rect_filled(Rect::from_center_size(dot, Vec2::splat(9.0 + h)), cr(2.5), r.color);
                }
                _ => {
                    p.circle_stroke(dot, 4.5 + h, Stroke::new(1.8, r.color));
                }
            }
            let tc = if r.collected { theme.text_faint } else if matches!(r.kind, Kind::SmallFiles | Kind::Hidden | Kind::Mount) { theme.text_dim } else { theme.text };
            let sz = fmt_size(r.size);
            let sw = p.layout_no_wrap(sz.clone(), bold(12.5), theme.text).size().x;
            let lw = rr.width() - 44.0 - sw - 12.0 - if r.denied { 60.0 } else { 0.0 };
            let lbl = widgets::truncate(p, &r.label, &font(13.0), lw);
            p.text(Pos2::new(rr.left() + 30.0, rr.center().y - 1.0), Align2::LEFT_CENTER, lbl, font(13.0), tc);
            if r.denied {
                p.text(Pos2::new(rr.right() - 16.0 - sw - 10.0, rr.center().y - 1.0), Align2::RIGHT_CENTER, "no access", font(11.0), theme.warn);
            }
            if r.collected {
                widgets::draw_icon(p, Icon::Acorn, Rect::from_center_size(Pos2::new(rr.right() - 16.0 - sw - 14.0, rr.center().y - 1.0), Vec2::splat(12.0)), theme.warn);
            }
            p.text(Pos2::new(rr.right() - 12.0, rr.center().y - 1.0), Align2::RIGHT_CENTER, sz, bold(12.5), lerp_color(theme.text_dim, theme.text, h));
            if resp.drag_started() && !matches!(r.kind, Kind::Hidden | Kind::Mount) {
                let at = resp.interact_pointer_pos().unwrap_or(rr.center());
                self.begin_drag(si, r.node, r.color, at);
            } else if resp.double_clicked() {
                action = Some((r.node, r.kind, 1));
            } else if resp.clicked() {
                action = Some((r.node, r.kind, 0));
            } else if resp.secondary_clicked() {
                action = Some((r.node, r.kind, 2));
            }
        }
        if let Some((n, rest, v)) = more {
            let rr = Rect::from_min_size(Pos2::new(inner.left() - 8.0, top + show as f32 * row_h), Vec2::new(inner.width() + 16.0, row_h - 2.0));
            let id = Id::new(("row-more", si));
            let resp = ui.interact(rr, id, Sense::click());
            let h = ui.ctx().animate_bool_with_time(id.with("h"), resp.hovered(), 0.1);
            let p = ui.painter();
            p.rect_filled(rr, cr(8.0), with_alpha(theme.surface_hi, h));
            p.text(Pos2::new(rr.left() + 30.0, rr.center().y), Align2::LEFT_CENTER, format!("{} more items…", fmt_count(n as u64)), font(13.0), theme.text_dim);
            p.text(Pos2::new(rr.right() - 12.0, rr.center().y), Align2::RIGHT_CENTER, fmt_size(rest), bold(12.5), theme.text_dim);
            if resp.clicked() {
                self.sessions[si].navigate(v);
                self.sfx(Sfx::Blip);
            }
        }
        if rows.is_empty() {
            let p = ui.painter();
            p.text(Pos2::new(inner.center().x, top + 40.0), Align2::CENTER_CENTER, if self.sessions[si].is_scanning() { "Scanning…" } else { "Empty" }, font(13.0), theme.text_faint);
        }

        // footer
        let p = ui.painter();
        let fy = inner.bottom() - footer_h + 8.0;
        p.line_segment([Pos2::new(inner.left(), fy - 6.0), Pos2::new(inner.right(), fy - 6.0)], Stroke::new(1.0, with_alpha(theme.stroke, 0.7)));
        if is_root_volume {
            if let Target::Volume(v) = &self.sessions[si].target {
                let free = if crate::scan::demo::enabled() { v.available } else { crate::scan::platform::volume_space(std::path::Path::new(&v.mount)).map(|x| x.1).unwrap_or(v.available) };
                let collected = self.sessions[si].collected_size();
                p.circle_filled(Pos2::new(inner.left() + 8.0, fy + 10.0), 4.0, theme.text_dim);
                p.text(Pos2::new(inner.left() + 22.0, fy + 10.0), Align2::LEFT_CENTER, "Free space", font(13.0), theme.text_dim);
                p.text(Pos2::new(inner.right() - 4.0, fy + 10.0), Align2::RIGHT_CENTER, fmt_size(free), bold(12.5), theme.text_dim);
                p.text(Pos2::new(inner.left() + 22.0, fy + 34.0), Align2::LEFT_CENTER, "Free + collected", font(13.0), theme.text_dim);
                p.text(Pos2::new(inner.left() + 4.0, fy + 34.0), Align2::LEFT_CENTER, "~", font(13.0), theme.text_dim);
                p.text(Pos2::new(inner.right() - 4.0, fy + 34.0), Align2::RIGHT_CENTER, fmt_size(free + collected), bold(12.5), theme.ok);
            }
        } else if let Some(path) = hovered_path {
            let pth = widgets::truncate(p, &path, &font(11.0), inner.width());
            p.text(Pos2::new(inner.left(), fy + 8.0), Align2::LEFT_CENTER, pth, font(11.0), theme.text_faint);
        }

        let s = &mut self.sessions[si];
        s.list_hover = new_hover;
        if let Some((node, kind, how)) = action {
            match (how, kind) {
                (2, _) => {
                    let pos = ui.ctx().input(|i| i.pointer.latest_pos()).unwrap_or(panel.center());
                    self.ctx_menu = Some(CtxMenu { session: si, node, pos, opened: Instant::now() });
                }
                (0, Kind::Dir) => {
                    s.navigate(View::node(node));
                    self.sfx(Sfx::Blip);
                }
                (0, Kind::SmallFiles) => self.expand(si, node),
                (1, Kind::File) => {
                    let path = s.tree.read().unwrap().path(node);
                    if s.source().is_local() {
                        super::app::os_open(&path, false);
                    }
                }
                _ => {}
            }
        }
    }

    fn collector_ui(&mut self, ui: &mut Ui, rect: Rect, si: usize) {
        let theme = self.theme.clone();
        let dragging = self.drag.as_ref().is_some_and(|d| d.session == si);
        let over = self.drag.as_ref().is_some_and(|d| d.over_bin);
        let deleting = self.sessions[si].delete.as_ref().is_some_and(|d| !d.finished.load(Ordering::Relaxed));
        let count = self.sessions[si].collector.len();
        let size = self.sessions[si].collected_size();
        self.bin_bounce = (self.bin_bounce - self.dt * 2.5).max(0.0);
        let dg = ui.ctx().animate_bool_with_time(Id::new("bin-drag"), dragging, 0.2);
        let ov = ui.ctx().animate_bool_with_time(Id::new("bin-over"), over, 0.12);
        let bounce = (self.bin_bounce * 14.0).sin() * self.bin_bounce * 0.03;
        let grow = 1.0 + 0.015 * ov + bounce;
        let r = Rect::from_center_size(rect.center(), rect.size() * grow);
        self.bin_rect = r;

        let id = Id::new(("collector", si));
        let resp = ui.interact(Rect::from_min_max(r.min, Pos2::new(r.right() - 118.0, r.bottom())), id, Sense::click());
        let h = ui.ctx().animate_bool_with_time(id.with("h"), resp.hovered(), 0.12);
        let p = ui.painter();
        if dg > 0.0 {
            let pulse = 0.5 + 0.5 * (self.time as f32 * 6.0).sin();
            widgets::glow(p, r.center(), r.width() * 0.6, with_alpha(theme.warn, (0.04 + 0.04 * pulse) * dg + 0.1 * ov));
        }
        widgets::shadow(p, r, 12.0, 0.7, &theme);
        p.rect_filled(r, cr(12.0), lerp_color(lerp_color(theme.surface_hi, lighten(theme.surface_hi, 0.04), h), lerp_color(theme.surface_hi, theme.warn, 0.35), ov));
        let border = lerp_color(with_alpha(theme.stroke, 0.8), theme.warn, dg);
        p.rect_stroke(r, cr(12.0), Stroke::new(1.0 + dg, border), egui::StrokeKind::Inside);
        let tx = r.left() + 20.0;
        if count == 0 {
            let msg = if dragging { "Drop here to collect" } else { "Drag slices or rows here to collect them" };
            let m = widgets::truncate(p, msg, &font(12.5), r.width() - 150.0);
            p.text(Pos2::new(tx, r.center().y), Align2::LEFT_CENTER, m, font(12.5), if dragging { theme.warn } else { theme.text_dim });
        } else {
            p.text(Pos2::new(tx, r.center().y - 8.0), Align2::LEFT_CENTER, fmt_size(size), display(16.0), theme.text);
            p.text(Pos2::new(tx, r.center().y + 11.0), Align2::LEFT_CENTER, format!("collected · {count} item{}", if count == 1 { "" } else { "s" }), font(11.5), theme.text_dim);
        }
        let btn = Rect::from_min_size(Pos2::new(r.right() - 112.0, r.center().y - 16.0), Vec2::new(100.0, 32.0));
        let del = widgets::button_ex(ui, btn, id.with("del"), "Delete", Some(Icon::Trash), BtnStyle::Danger, &theme, count > 0 && !deleting);
        self.mark("delete", btn);
        self.mark("bin", r);
        if del.clicked() {
            self.open_delete_confirm(si);
        }
        if resp.clicked() && count > 0 {
            self.bin_open = !self.bin_open;
        }
        if count == 0 {
            self.bin_open = false;
        }
        if self.bin_open {
            self.collector_popover(ui.ctx(), r, si);
        }
    }

    fn collector_popover(&mut self, ctx: &egui::Context, anchor: Rect, si: usize) {
        let theme = self.theme.clone();
        let items: Vec<(u32, String, u64, String)> = {
            let s = &self.sessions[si];
            let t = s.tree.read().unwrap();
            s.collector.iter().filter(|n| t.is_alive(**n)).map(|n| (*n, node_label(&t, *n), t.get(*n).size, t.path(*n))).collect()
        };
        let row_h = 40.0;
        let h = (items.len().min(8) as f32) * row_h + 52.0;
        let rect = Rect::from_min_size(Pos2::new(anchor.left(), anchor.top() - h - 10.0), Vec2::new(anchor.width(), h));
        let mut remove = None;
        let mut close = false;
        egui::Area::new(Id::new("collector-pop")).order(egui::Order::Foreground).fixed_pos(rect.min).show(ctx, |ui| {
            let (resp_rect, _) = ui.allocate_exact_size(rect.size(), Sense::click());
            let p = ui.painter();
            widgets::shadow(p, resp_rect, 16.0, 1.0, &theme);
            p.rect_filled(resp_rect, cr(16.0), theme.surface_hi);
            p.rect_stroke(resp_rect, cr(16.0), Stroke::new(1.0, theme.stroke), egui::StrokeKind::Inside);
            p.text(resp_rect.left_top() + Vec2::new(16.0, 18.0), Align2::LEFT_CENTER, "Collected for deletion", bold(13.0), theme.text);
            let clear = Rect::from_min_size(Pos2::new(resp_rect.right() - 86.0, resp_rect.top() + 6.0), Vec2::new(74.0, 24.0));
            if widgets::button(ui, clear, Id::new("clear-bin"), "Clear", None, BtnStyle::Ghost, &theme).clicked() {
                close = true;
            }
            for (k, (node, label, size, path)) in items.iter().take(8).enumerate() {
                let rr = Rect::from_min_size(Pos2::new(resp_rect.left() + 8.0, resp_rect.top() + 40.0 + k as f32 * row_h), Vec2::new(resp_rect.width() - 16.0, row_h - 4.0));
                let id = Id::new(("bin-row", *node));
                let resp = ui.interact(rr, id, Sense::hover());
                let hh = ui.ctx().animate_bool_with_time(id, resp.hovered(), 0.1);
                let p = ui.painter();
                p.rect_filled(rr, cr(10.0), with_alpha(theme.surface, hh));
                let lw = rr.width() - 140.0;
                p.text(Pos2::new(rr.left() + 12.0, rr.top() + 12.0), Align2::LEFT_CENTER, widgets::truncate(p, label, &bold(12.5), lw), bold(12.5), theme.text);
                p.text(Pos2::new(rr.left() + 12.0, rr.top() + 27.0), Align2::LEFT_CENTER, widgets::truncate(p, path, &font(10.5), lw), font(10.5), theme.text_faint);
                p.text(Pos2::new(rr.right() - 44.0, rr.center().y), Align2::RIGHT_CENTER, fmt_size(*size), bold(12.5), theme.text_dim);
                let x = Rect::from_center_size(Pos2::new(rr.right() - 20.0, rr.center().y), Vec2::splat(24.0));
                if widgets::icon_button(ui, x, id.with("x"), Icon::Close, &theme, true).clicked() {
                    remove = Some(*node);
                }
            }
        });
        let s = &mut self.sessions[si];
        if let Some(n) = remove {
            s.collector.retain(|c| *c != n);
        }
        if close {
            s.collector.clear();
            self.bin_open = false;
        }
        // click elsewhere closes
        if ctx.input(|i| i.pointer.any_pressed()) {
            if let Some(p) = ctx.input(|i| i.pointer.interact_pos()) {
                if !rect.contains(p) && !anchor.contains(p) {
                    self.bin_open = false;
                }
            }
        }
    }

    pub fn open_delete_confirm(&mut self, si: usize) {
        let s = &self.sessions[si];
        let t = s.tree.read().unwrap();
        let rules = match &t.source {
            crate::tree::Source::Local => Rules::current(),
            _ => Rules::for_os(Os::Linux, None),
        };
        let is_rclone = matches!(t.source, crate::tree::Source::Rclone { .. });
        let items: Vec<DeleteItem> = s
            .collector
            .iter()
            .filter(|n| t.is_alive(**n))
            .map(|n| {
                let path = t.path(*n);
                let verdict = if is_rclone { crate::safety::Verdict::Ok } else { rules.check(&path) };
                DeleteItem { node: *n, name: node_label(&t, *n), size: t.get(*n).size, path, verdict }
            })
            .collect();
        let local = t.source.is_local();
        drop(t);
        let backup_folder = self.settings.backup_folder.clone().unwrap_or_default();
        let backup_remote = self.settings.backup_remote.clone().unwrap_or_default();
        self.bin_open = false;
        self.open_modal(Modal::ConfirmDelete { session: si, items, mode: if local { 0 } else { 1 }, backup_folder, backup_remote, ack: false });
    }

    pub fn context_menu_ui(&mut self, ctx: &egui::Context) {
        let Some(m) = &self.ctx_menu else { return };
        let (si, node, pos, opened) = (m.session, m.node, m.pos, m.opened);
        let theme = self.theme.clone();
        let Some(s) = self.sessions.get(si) else {
            self.ctx_menu = None;
            return;
        };
        let (path, kind, local, name) = {
            let t = s.tree.read().unwrap();
            if !t.is_alive(node) {
                self.ctx_menu = None;
                return;
            }
            (t.path(node), t.get(node).kind, t.source.is_local(), node_label(&t, node))
        };
        let mut items: Vec<(&str, Icon, u8)> = Vec::new();
        if kind == Kind::Dir {
            items.push(("Zoom in", Icon::Eye, 0));
        }
        if local && kind != Kind::SmallFiles {
            items.push(("Open", Icon::File, 1));
            items.push((if cfg!(target_os = "macos") { "Reveal in Finder" } else { "Show in file manager" }, Icon::Folder, 2));
        }
        items.push(("Copy path", Icon::File, 3));
        if !matches!(kind, Kind::Hidden | Kind::Mount) {
            items.push(("Collect for deletion", Icon::Acorn, 4));
        }
        let w = 230.0;
        let h = items.len() as f32 * 32.0 + 44.0;
        let screen = ctx.content_rect();
        let mut origin = pos;
        origin.x = origin.x.min(screen.right() - w - 8.0);
        origin.y = origin.y.min(screen.bottom() - h - 8.0);
        let appear = (opened.elapsed().as_secs_f32() / 0.12).min(1.0);
        let mut chosen = None;
        let mut rect_out = Rect::NOTHING;
        egui::Area::new(Id::new("ctx-menu")).order(egui::Order::Foreground).fixed_pos(origin).show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
            rect_out = rect;
            let p = ui.painter();
            widgets::shadow(p, rect, 12.0, appear, &theme);
            p.rect_filled(rect, cr(12.0), with_alpha(theme.surface_hi, appear));
            p.rect_stroke(rect, cr(12.0), Stroke::new(1.0, with_alpha(theme.stroke, appear)), egui::StrokeKind::Inside);
            let n = widgets::truncate(p, &name, &bold(12.0), w - 28.0);
            p.text(rect.left_top() + Vec2::new(14.0, 20.0), Align2::LEFT_CENTER, n, bold(12.0), with_alpha(theme.text_dim, appear));
            for (k, (label, icon, act)) in items.iter().enumerate() {
                let rr = Rect::from_min_size(rect.left_top() + Vec2::new(6.0, 38.0 + k as f32 * 32.0), Vec2::new(w - 12.0, 30.0));
                let id = Id::new(("ctx", k));
                let resp = ui.interact(rr, id, Sense::click());
                let hh = ui.ctx().animate_bool_with_time(id, resp.hovered(), 0.08);
                let p = ui.painter();
                p.rect_filled(rr, cr(8.0), with_alpha(theme.accent, 0.85 * hh));
                widgets::draw_icon(p, *icon, Rect::from_center_size(Pos2::new(rr.left() + 16.0, rr.center().y), Vec2::splat(14.0)), lerp_color(theme.text_dim, Color32::WHITE, hh));
                p.text(Pos2::new(rr.left() + 34.0, rr.center().y), Align2::LEFT_CENTER, *label, font(13.0), lerp_color(theme.text, Color32::WHITE, hh));
                if resp.clicked() {
                    chosen = Some(*act);
                }
            }
        });
        if let Some(a) = chosen {
            self.ctx_menu = None;
            match a {
                0 => {
                    self.sessions[si].navigate(View::node(node));
                    self.sfx(Sfx::Blip);
                }
                1 => super::app::os_open(&path, false),
                2 => super::app::os_open(&path, true),
                3 => {
                    ctx.copy_text(path);
                    self.toasts.push("Path copied".into(), theme.accent);
                }
                4 => {
                    if self.sessions[si].collect(node) {
                        self.sfx(Sfx::Plop);
                        self.bin_bounce = 1.0;
                    }
                }
                _ => {}
            }
            return;
        }
        if opened.elapsed().as_millis() > 150 && ctx.input(|i| i.pointer.any_pressed()) {
            if let Some(p) = ctx.input(|i| i.pointer.interact_pos()) {
                if !rect_out.contains(p) {
                    self.ctx_menu = None;
                }
            }
        }
        let _ = TAU;
    }
}

/// CPU fallback when OpenGL shaders are unavailable.
fn cpu_mesh(painter: &egui::Painter, verts: &[f32], idx: &[u32]) {
    let mut mesh = egui::Mesh::default();
    for v in verts.chunks_exact(sunburst::VERT_FLOATS) {
        let a = v[5];
        let shade = 1.0;
        let c = Color32::from_rgba_premultiplied(
            (v[2] * shade * a * 255.0) as u8,
            (v[3] * shade * a * 255.0) as u8,
            (v[4] * shade * a * 255.0) as u8,
            (a * 255.0) as u8,
        );
        mesh.colored_vertex(Pos2::new(v[0], v[1]), c);
    }
    mesh.indices = idx.to_vec();
    painter.add(Shape::mesh(mesh));
}

#[allow(dead_code)]
fn _unused(_: &Theme) {}
