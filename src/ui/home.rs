//! Home screen: disks, recent scans and other sources.

use super::app::{App, Modal, Screen, Target};
use super::theme::{lerp_color, lighten, with_alpha};
use super::widgets::{self, bold, cr, font, BtnStyle, Icon};
use crate::i18n::{tr, tr_status, trf};
use crate::tree::fmt_size;
use crate::update::State as UpState;
use eframe::egui::{self, Align2, Color32, Id, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use std::sync::atomic::Ordering;

pub const TITLEBAR_INSET: f32 = if cfg!(target_os = "macos") { 28.0 } else { 0.0 };

impl App {
    /// Header shared by all screens; returns the remaining rect below it.
    pub fn window_drag_area(&self, ui: &mut Ui, rect: Rect) {
        let resp = ui.interact(rect, Id::new("window-drag").with(rect.min.x as i32), Sense::click_and_drag());
        if resp.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if resp.double_clicked() {
            let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
        }
    }

    pub fn home_ui(&mut self, ui: &mut Ui, screen: Rect) {
        let theme = self.theme.clone();
        let t = self.time;
        self.window_drag_area(ui, Rect::from_min_size(screen.min, Vec2::new(screen.width(), 90.0 + TITLEBAR_INSET)));


        let w = (screen.width() - 64.0).min(820.0);
        let x0 = screen.center().x - w / 2.0;
        // The page scrolls when the window is shorter than the content.
        let max_scroll = (self.home_content_h - screen.height()).max(0.0);
        if self.modal.is_none() && ui.rect_contains_pointer(screen) {
            self.home_scroll -= ui.input(|i| i.smooth_scroll_delta.y);
        }
        self.home_scroll = self.home_scroll.clamp(0.0, max_scroll);
        let page_top = screen.top() - self.home_scroll;
        let mut y = page_top + TITLEBAR_INSET + 24.0;

        // --- header
        {
            let p = ui.painter();
            let _ = t;
            let logo = Rect::from_min_size(Pos2::new(x0, y + 2.0), Vec2::splat(46.0));
            let tex = self.logo.get_or_insert_with(|| {
                let px = 184;
                let pm = crate::icon::render(px as u32, crate::icon::Frame::IDLE, false);
                let img = egui::ColorImage::from_rgba_unmultiplied([px, px], &crate::icon::rgba(&pm));
                ui.ctx().load_texture("app-icon", img, egui::TextureOptions::LINEAR)
            });
            p.image(tex.id(), logo, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            p.text(Pos2::new(x0 + 58.0, y + 8.0), Align2::LEFT_TOP, "SquirrelDisk", widgets::brand(26.0), theme.text);
            p.text(Pos2::new(x0 + 59.0, y + 40.0), Align2::LEFT_TOP, tr("See what's using your disk space, and clean it up safely."), font(13.0), theme.text_dim);
        }
        let gear = Rect::from_center_size(Pos2::new(x0 + w - 18.0, y + 26.0), Vec2::splat(36.0));
        if widgets::icon_button(ui, gear, Id::new("settings"), Icon::Gear, &theme, true).clicked() {
            self.open_modal(Modal::Settings);
        }
        self.update_pill(ui, Pos2::new(gear.left() - 12.0, gear.center().y));
        y += 92.0;

        // --- disks
        section_label(ui, Pos2::new(x0, y), tr("DISKS"), &theme);
        y += 22.0;
        let volumes = self.volumes.clone();
        for (i, v) in volumes.iter().enumerate() {
            let rect = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(w, 76.0));
            self.volume_card(ui, rect, i, v);
            y += 86.0;
        }
        if volumes.is_empty() {
            ui.painter().text(Pos2::new(x0, y + 10.0), Align2::LEFT_TOP, tr("No disks found"), font(13.0), theme.text_faint);
            y += 40.0;
        }

        // --- other sessions (folders, servers, cloud)
        let others: Vec<usize> = (0..self.sessions.len()).filter(|i| !matches!(self.sessions[*i].target, Target::Volume(_))).collect();
        if !others.is_empty() {
            y += 8.0;
            section_label(ui, Pos2::new(x0, y), tr("RECENT SCANS"), &theme);
            y += 22.0;
            for i in others {
                let rect = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(w, 64.0));
                self.session_card(ui, rect, i);
                y += 74.0;
            }
        }

        // --- add a place
        y += 8.0;
        section_label(ui, Pos2::new(x0, y), tr("SCAN SOMETHING ELSE"), &theme);
        y += 22.0;
        let gap = 14.0;
        let cw = (w - gap * 2.0) / 3.0;
        let rclone_state = self.rclone.lock().unwrap().clone();
        let rclone_sub = match &rclone_state {
            None => tr("Looking for rclone…").to_string(),
            Some(v) if v.first().map(|s| s == "\u{0}").unwrap_or(false) => tr("S3, Drive, FTP… (needs rclone)").to_string(),
            Some(v) => if v.len() == 1 { tr("1 remote configured").to_string() } else { trf("{n} remotes configured", &[("n", &v.len())]) },
        };
        let cards = [
            (Icon::Folder, tr("A folder"), tr("Pick any folder, or drop it here").to_string(), 0usize),
            (Icon::Server, tr("Remote server"), tr("Any machine you can SSH into").to_string(), 1),
            (Icon::Cloud, tr("Cloud storage"), rclone_sub, 2),
        ];
        for (k, (icon, title, sub, action)) in cards.into_iter().enumerate() {
            let rect = Rect::from_min_size(Pos2::new(x0 + k as f32 * (cw + gap), y), Vec2::new(cw, 104.0));
            if source_card(ui, rect, Id::new(("src", k)), icon, title, &sub, &theme).clicked() {
                match action {
                    0 => {
                        if let Some(p) = rfd::FileDialog::new().set_title(tr("Choose a folder to scan")).pick_folder() {
                            self.start_session(Target::Folder(p));
                        }
                    }
                    1 => {
                        let host = self.settings.ssh_history.first().cloned().unwrap_or_default();
                        self.open_modal(Modal::Ssh { host, path: "/".into() });
                    }
                    _ => self.open_modal(Modal::Rclone { path: String::new() }),
                }
            }
        }
        y += 118.0;

        let ad_rect = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(w, 52.0));
        self.sponsor_banner(ui, ad_rect, "home");
        y += 64.0;

        let p = ui.painter();
        let foot = format!("v{} · {}", env!("CARGO_PKG_VERSION"), tr("drop a folder anywhere to scan it · Esc goes back"));
        let foot_y = (y + 20.0).max(screen.bottom() - 26.0 - self.home_scroll);
        p.text(Pos2::new(screen.center().x, foot_y), Align2::CENTER_CENTER, foot, font(11.5), theme.text_faint);
        self.home_content_h = y + 44.0 - page_top;
        if max_scroll > 0.0 {
            let track = Rect::from_min_max(Pos2::new(screen.right() - 9.0, screen.top() + TITLEBAR_INSET + 8.0), Pos2::new(screen.right() - 4.0, screen.bottom() - 8.0));
            let thumb_h = (track.height() * screen.height() / self.home_content_h).max(32.0);
            let t = self.home_scroll / max_scroll;
            let thumb = Rect::from_min_size(Pos2::new(track.left(), track.top() + (track.height() - thumb_h) * t), Vec2::new(track.width(), thumb_h));
            let resp = ui.interact(thumb.expand2(Vec2::new(4.0, 0.0)), Id::new("home-scroll"), Sense::drag());
            if resp.dragged() {
                self.home_scroll = (self.home_scroll + resp.drag_delta().y * max_scroll / (track.height() - thumb_h).max(1.0)).clamp(0.0, max_scroll);
            }
            let h = ui.ctx().animate_bool_with_time(Id::new("home-scroll-h"), resp.hovered() || resp.dragged(), 0.12);
            let p = ui.painter();
            p.rect_filled(track, cr(3.0), with_alpha(theme.stroke, 0.35));
            p.rect_filled(thumb, cr(3.0), lerp_color(with_alpha(theme.text_faint, 0.8), theme.text_dim, h));
        }
    }

    pub fn update_pill(&mut self, ui: &mut Ui, right_center: Pos2) {
        let theme = self.theme.clone();
        let state = self.updater.state();
        let (label, style, icon) = match &state {
            UpState::Available(r) => (trf("Update to {version}", &[("version", &r.version)]), BtnStyle::Primary, Icon::Download),
            UpState::Downloading => {
                let d = self.updater.downloaded.load(Ordering::Relaxed);
                let tot = self.updater.total.load(Ordering::Relaxed).max(1);
                (trf("Downloading {percent}%", &[("percent", &(d * 100 / tot))]), BtnStyle::Subtle, Icon::Download)
            }
            UpState::Ready(_) => (tr("Restart to update").to_string(), BtnStyle::Primary, Icon::Refresh),
            _ => return,
        };
        let w = ui.painter().layout_no_wrap(label.clone(), bold(13.0), Color32::WHITE).size().x + 50.0;
        let rect = Rect::from_min_size(Pos2::new(right_center.x - w, right_center.y - 16.0), Vec2::new(w, 32.0));
        if widgets::button(ui, rect, Id::new("update-pill"), &label, Some(icon), style, &theme).clicked() {
            match state {
                UpState::Available(r) => self.updater.install(r, ui.ctx().clone()),
                UpState::Ready(_) => crate::update::restart(),
                _ => {}
            }
        }
    }

    fn volume_card(&mut self, ui: &mut Ui, rect: Rect, i: usize, v: &crate::disks::Volume) {
        let theme = self.theme.clone();
        let id = Id::new(("vol", &v.mount));
        let resp = ui.interact(rect, id, Sense::click());
        let h = ui.ctx().animate_bool_with_time(id.with("h"), resp.hovered(), 0.15);
        let session = self.sessions.iter().position(|s| matches!(&s.target, Target::Volume(x) if x.mount == v.mount));
        let r = rect;
        {
            let p = ui.painter();
            widgets::shadow(p, r, 10.0, 0.5, &theme);
            p.rect_filled(r, cr(10.0), lerp_color(with_alpha(theme.surface, 0.85), theme.surface_hi, h * 0.6));
            p.rect_stroke(r, cr(10.0), Stroke::new(1.0, with_alpha(theme.stroke, 0.7)), egui::StrokeKind::Inside);
            // icon tile
            let tile = Rect::from_min_size(r.min + Vec2::new(14.0, 16.0), Vec2::splat(44.0));
            let tc = if v.is_boot { theme.accent } else if v.removable { theme.warn } else { theme.accent2 };
            p.rect_filled(tile, cr(8.0), tc);
            widgets::draw_icon(p, if v.removable { Icon::External } else { Icon::Disk }, tile.shrink(10.0), Color32::WHITE);
            // texts
            p.text(Pos2::new(tile.right() + 14.0, r.top() + 26.0), Align2::LEFT_CENTER, &v.name, bold(15.0), theme.text);
            let kind = if v.is_boot { tr("startup disk") } else if v.removable { tr("removable") } else { tr("volume") };
            let sub = format!("{} · {} · {} · {}", fmt_size(v.total), kind, v.fs, v.mount);
            let sub = widgets::truncate(p, &sub, &font(12.0), r.width() - 480.0);
            p.text(Pos2::new(tile.right() + 14.0, r.top() + 48.0), Align2::LEFT_CENTER, sub, font(12.0), theme.text_dim);
        }

        // right side: bar + button(s)
        let btn = Rect::from_min_size(Pos2::new(r.right() - 118.0, r.center().y - 16.0), Vec2::new(104.0, 32.0));
        // leave room for the small refresh / cancel button that sits left of `btn`
        let bar = Rect::from_min_size(Pos2::new(btn.left() - 48.0 - 190.0, r.center().y - 10.0), Vec2::new(190.0, 8.0));
        let p = ui.painter().clone();
        match session.map(|s| &self.sessions[s]) {
            Some(s) if s.is_scanning() => {
                let frac = s.progress.fraction();
                widgets::progress_bar(&p, bar, frac, &theme, self.time, None);
                let files = s.progress.files.load(Ordering::Relaxed);
                let txt = match frac {
                    Some(f) => trf("scanning · {percent}% · {files} files", &[("percent", &format!("{:.0}", f * 100.0)), ("files", &crate::tree::fmt_count_compact(files))]),
                    None => trf("scanning · {files} files", &[("files", &crate::tree::fmt_count_compact(files))]),
                };
                p.text(Pos2::new(bar.left(), bar.bottom() + 12.0), Align2::LEFT_CENTER, txt, font(11.5), theme.text_dim);
                if widgets::button(ui, btn, id.with("view"), tr("Watch"), Some(Icon::Eye), BtnStyle::Subtle, &theme).clicked() {
                    self.screen = Screen::Session(session.unwrap());
                }
                let x = Rect::from_center_size(Pos2::new(btn.left() - 22.0, btn.center().y), Vec2::splat(28.0));
                if widgets::icon_button(ui, x, id.with("cancel"), Icon::Close, &theme, true).clicked() {
                    self.cancel_session(session.unwrap());
                }
            }
            other => {
                let used = v.used_frac();
                let col = if used > 0.9 { theme.danger } else if used > 0.75 { theme.warn } else { theme.ok };
                widgets::progress_bar(&p, bar, Some(used), &theme, 0.25, Some(col));
                p.text(Pos2::new(bar.right(), bar.bottom() + 12.0), Align2::RIGHT_CENTER, trf("{size} free", &[("size", &fmt_size(v.available))]), bold(12.0), col);
                p.text(Pos2::new(bar.left(), bar.bottom() + 12.0), Align2::LEFT_CENTER, trf("{size} used", &[("size", &fmt_size(v.used()))]), font(11.5), theme.text_faint);
                if other.is_some() {
                    if widgets::button(ui, btn, id.with("view"), tr("View"), Some(Icon::Eye), BtnStyle::Primary, &theme).clicked() || resp.clicked() {
                        self.screen = Screen::Session(session.unwrap());
                        self.sfx(crate::sound::Sfx::Blip);
                    }
                    let rr = Rect::from_center_size(Pos2::new(btn.left() - 22.0, btn.center().y), Vec2::splat(28.0));
                    if widgets::icon_button(ui, rr, id.with("rescan"), Icon::Refresh, &theme, true).clicked() {
                        self.start_session(Target::Volume(v.clone()));
                    }
                } else if {
                    self.mark(format!("scan:{}", v.mount), btn);
                    widgets::button(ui, btn, id.with("scan"), tr("Scan"), Some(Icon::Bolt), BtnStyle::Primary, &theme).clicked() || resp.clicked()
                } {
                    self.start_session(Target::Volume(v.clone()));
                }
            }
        }
        let _ = i;
    }

    fn session_card(&mut self, ui: &mut Ui, rect: Rect, i: usize) {
        let theme = self.theme.clone();
        let id = Id::new(("sess", i));
        let resp = ui.interact(rect, id, Sense::click());
        let h = ui.ctx().animate_bool_with_time(id.with("h"), resp.hovered(), 0.15);
        let s = &self.sessions[i];
        let (icon, kind) = match &s.target {
            Target::Folder(_) => (Icon::Folder, tr("folder")),
            Target::Ssh { .. } => (Icon::Server, tr("server")),
            Target::Rclone(_) => (Icon::Cloud, tr("cloud")),
            Target::Volume(_) => (Icon::Disk, tr("disk")),
        };
        let size = {
            let t = s.tree.read().unwrap();
            t.get(t.root).size
        };
        let status = if let Some(e) = s.progress.error.lock().unwrap().clone() {
            trf("failed: {error}", &[("error", &e)])
        } else if s.is_scanning() {
            let st = s.progress.status.lock().unwrap().clone();
            trf("{status}… {files} files", &[("status", &tr_status(&st)), ("files", &crate::tree::fmt_count_compact(s.progress.files.load(Ordering::Relaxed)))])
        } else {
            format!("{} · {}", kind, fmt_size(size))
        };
        let scanning = s.is_scanning();
        let title = s.title.clone();
        {
            let p = ui.painter();
            let r = rect;
            p.rect_filled(r, cr(10.0), lerp_color(with_alpha(theme.surface, 0.7), theme.surface_hi, h * 0.6));
            let tile = Rect::from_min_size(r.min + Vec2::new(12.0, 12.0), Vec2::splat(40.0));
            p.rect_filled(tile, cr(8.0), with_alpha(theme.accent, 0.25));
            widgets::draw_icon(p, icon, tile.shrink(10.0), theme.accent);
            let t2 = widgets::truncate(p, &title, &bold(14.0), r.width() - 260.0);
            p.text(Pos2::new(tile.right() + 12.0, r.top() + 22.0), Align2::LEFT_CENTER, t2, bold(14.0), theme.text);
            let st = widgets::truncate(p, &status, &font(12.0), r.width() - 260.0);
            p.text(Pos2::new(tile.right() + 12.0, r.top() + 42.0), Align2::LEFT_CENTER, st, font(12.0), theme.text_dim);
        }
        let btn = Rect::from_min_size(Pos2::new(rect.right() - 118.0, rect.center().y - 16.0), Vec2::new(104.0, 32.0));
        if widgets::button(ui, btn, id.with("v"), if scanning { tr("Watch") } else { tr("View") }, Some(Icon::Eye), BtnStyle::Primary, &theme).clicked() || resp.clicked() {
            self.screen = Screen::Session(i);
        }
        let x = Rect::from_center_size(Pos2::new(btn.left() - 22.0, btn.center().y), Vec2::splat(28.0));
        if widgets::icon_button(ui, x, id.with("x"), Icon::Close, &theme, true).clicked() {
            self.cancel_session(i);
        }
    }
}

pub fn section_label(ui: &Ui, pos: Pos2, text: &str, theme: &super::theme::Theme) {
    ui.painter().text(pos, Align2::LEFT_TOP, text, bold(11.0), theme.text_faint);
}

fn source_card(ui: &mut Ui, rect: Rect, id: Id, icon: Icon, title: &str, sub: &str, theme: &super::theme::Theme) -> egui::Response {
    let resp = ui.interact(rect, id, Sense::click());
    let h = ui.ctx().animate_bool_with_time(id.with("h"), resp.hovered(), 0.15);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let p = ui.painter();
    let r = rect;
    
    p.rect_filled(r, cr(10.0), lerp_color(with_alpha(theme.surface, 0.8), theme.surface_hi, h));
    p.rect_stroke(r, cr(10.0), Stroke::new(1.0, lerp_color(with_alpha(theme.stroke, 0.6), theme.accent, h * 0.7)), egui::StrokeKind::Inside);
    let ic = Pos2::new(r.left() + 30.0, r.top() + 32.0);
    p.rect_filled(Rect::from_center_size(ic, Vec2::splat(34.0)), cr(8.0), with_alpha(lighten(theme.accent, 0.05), 0.18 + 0.1 * h));
    widgets::draw_icon(p, icon, Rect::from_center_size(ic, Vec2::splat(20.0)), lerp_color(theme.accent, theme.text, h * 0.5));
    p.text(Pos2::new(r.left() + 16.0, r.top() + 66.0), Align2::LEFT_CENTER, title, bold(14.0), theme.text);
    let s = widgets::truncate(p, sub, &font(11.5), r.width() - 28.0);
    p.text(Pos2::new(r.left() + 16.0, r.top() + 86.0), Align2::LEFT_CENTER, s, font(11.5), theme.text_dim);
    resp
}
