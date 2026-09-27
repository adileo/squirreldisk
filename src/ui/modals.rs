//! Custom modal dialogs.

use super::app::{App, Modal, Target};
use super::theme::{lerp_color, with_alpha, Theme};
use super::widgets::{self, bold, cr, display, font, BtnStyle, Icon};
use crate::delete::Mode;
use crate::safety::Verdict;
use crate::sound::Sfx;
use crate::tree::fmt_size;
use crate::update::State as UpState;
use eframe::egui::{self, Align2, Color32, Id, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};
use std::f32::consts::TAU;
use std::sync::atomic::Ordering;
use std::time::Instant;

thread_local! {
    /// Rects registered inside modal closures (which can't borrow `App`).
    static MARKS: std::cell::RefCell<Vec<(String, Rect)>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn mark(name: impl Into<String>, rect: Rect) {
    MARKS.with(|m| m.borrow_mut().push((name.into(), rect)));
}

/// Draws backdrop + card; returns whether the backdrop was clicked.
fn modal_frame(ctx: &egui::Context, opened: Instant, size: Vec2, theme: &Theme, add: impl FnOnce(&mut Ui, Rect)) -> bool {
    let screen = ctx.content_rect();
    let t = (opened.elapsed().as_secs_f32() / 0.18).min(1.0);
    let e = 1.0 - (1.0 - t).powi(3);
    let mut backdrop_clicked = false;
    egui::Area::new(Id::new("modal")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let resp = ui.allocate_rect(screen, Sense::click());
        ui.painter().rect_filled(screen, cr(0.0), with_alpha(Color32::from_rgb(8, 5, 14), 0.55 * e));
        let size = Vec2::new(size.x.min(screen.width() - 32.0), size.y.min(screen.height() - 32.0));
        let card = Rect::from_center_size(screen.center() + Vec2::new(0.0, (1.0 - e) * 8.0), size);
        let p = ui.painter();
        widgets::shadow(p, card, 14.0, 1.3 * e, theme);
        p.rect_filled(card, cr(14.0), with_alpha(lerp_color(theme.surface, theme.bg_top, 0.3), e));
        p.rect_stroke(card, cr(14.0), Stroke::new(1.0, with_alpha(theme.stroke, e)), egui::StrokeKind::Inside);
        // gentle accent glow at the top
        let p2 = p.with_clip_rect(card);
        let _ = &p2;
        if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                backdrop_clicked = !card.contains(pos);
            }
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(card), |ui| add(ui, card));
    });
    backdrop_clicked
}

fn title(ui: &Ui, card: Rect, text: &str, sub: Option<&str>, theme: &Theme) {
    let p = ui.painter();
    p.text(card.left_top() + Vec2::new(28.0, 26.0), Align2::LEFT_TOP, text, display(19.0), theme.text);
    if let Some(s) = sub {
        p.text(card.left_top() + Vec2::new(28.0, 58.0), Align2::LEFT_TOP, s, font(12.5), theme.text_dim);
    }
}

fn close_button(ui: &mut Ui, card: Rect, theme: &Theme) -> bool {
    let r = Rect::from_center_size(Pos2::new(card.right() - 30.0, card.top() + 32.0), Vec2::splat(30.0));
    mark("modal-close", r);
    widgets::icon_button(ui, r, Id::new("modal-close"), Icon::Close, theme, true).clicked()
}

fn wrapped(ui: &Ui, pos: Pos2, width: f32, text: &str, f: egui::FontId, color: Color32) -> f32 {
    let g = ui.painter().layout(text.to_string(), f, color, width);
    let h = g.size().y;
    ui.painter().galley(pos, g, color);
    h
}

impl App {
    pub fn modal_ui(&mut self, ctx: &egui::Context) {
        let Some(modal) = self.modal.take() else { return };
        MARKS.with(|m| m.borrow_mut().clear());
        let keep = match modal {
            Modal::Settings => self.settings_modal(ctx),
            Modal::Ssh { host, path } => self.ssh_modal(ctx, host, path),
            Modal::Rclone { path } => self.rclone_modal(ctx, path),
            Modal::ConfirmDelete { session, items, mode, backup_folder, backup_remote, ack } => {
                self.confirm_delete_modal(ctx, session, items, mode, backup_folder, backup_remote, ack)
            }
            Modal::Deleting { session, finished_at } => self.deleting_modal(ctx, session, finished_at),
            Modal::Error { title: t, message } => self.error_modal(ctx, t, message),
            Modal::SshAuth { host, path, secret, retry } => self.ssh_auth_modal(ctx, host, path, secret, retry),
        };
        // a modal may have opened another one
        if self.modal.is_none() {
            self.modal = keep;
        }
        for (name, rect) in MARKS.with(|m| std::mem::take(&mut *m.borrow_mut())) {
            self.mark(name, rect);
        }
    }

    fn ssh_auth_modal(&mut self, ctx: &egui::Context, host: String, path: String, mut secret: String, retry: bool) -> Option<Modal> {
        let theme = self.theme.clone();
        let mut close = false;
        let mut go = false;
        let bd = modal_frame(ctx, self.modal_opened, Vec2::new(500.0, 290.0), &theme, |ui, card| {
            title(ui, card, "Authentication needed", Some(&format!("{host} needs your SSH key passphrase or password.")), &theme);
            close = close_button(ui, card, &theme);
            let x0 = card.left() + 28.0;
            let w = card.width() - 56.0;
            let mut y = card.top() + 96.0;
            ui.painter().text(Pos2::new(x0, y), Align2::LEFT_TOP, "PASSPHRASE OR PASSWORD", bold(11.0), theme.text_faint);
            y += 18.0;
            let r = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(w, 38.0));
            let id = Id::new("ssh-secret");
            {
                let p = ui.painter();
                p.rect_filled(r, cr(6.0), with_alpha(theme.bg_bottom, 0.9));
                p.rect_stroke(r, cr(6.0), Stroke::new(1.0, if retry { theme.danger } else { theme.accent }), egui::StrokeKind::Inside);
            }
            let edit = egui::TextEdit::singleline(&mut secret)
                .id(id)
                .password(true)
                .frame(egui::Frame::NONE)
                .font(font(14.0))
                .text_color(theme.text)
                .desired_width(w - 24.0)
                .vertical_align(egui::Align::Center);
            let resp = ui.put(r.shrink2(Vec2::new(12.0, 0.0)), edit);
            if !resp.has_focus() && secret.is_empty() {
                resp.request_focus();
            }
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            y += 48.0;
            let note = if retry { "That didn't work, please try again." } else { "Kept in memory for this session only, never saved to disk." };
            ui.painter().text(Pos2::new(x0, y), Align2::LEFT_TOP, note, font(12.0), if retry { theme.danger } else { theme.text_faint });
            let b = Rect::from_min_size(Pos2::new(card.right() - 150.0, card.bottom() - 60.0), Vec2::new(122.0, 36.0));
            go |= widgets::button_ex(ui, b, Id::new("auth-go"), "Connect", Some(Icon::Bolt), BtnStyle::Primary, &theme, !secret.is_empty()).clicked();
            let c = Rect::from_min_size(Pos2::new(card.right() - 260.0, card.bottom() - 60.0), Vec2::new(100.0, 36.0));
            close |= widgets::button(ui, c, Id::new("auth-cancel"), "Cancel", None, BtnStyle::Ghost, &theme).clicked();
        });
        if go && !secret.is_empty() {
            crate::scan::remote::set_secret(&host, secret);
            self.modal = None;
            self.start_session(Target::Ssh { host, path });
            return None;
        }
        if close || bd {
            self.ssh_tried.remove(&host);
            None
        } else {
            Some(Modal::SshAuth { host, path, secret, retry })
        }
    }

    fn error_modal(&mut self, ctx: &egui::Context, t: String, message: String) -> Option<Modal> {
        let theme = self.theme.clone();
        let mut close = false;
        let bd = modal_frame(ctx, self.modal_opened, Vec2::new(460.0, 220.0), &theme, |ui, card| {
            title(ui, card, &t, None, &theme);
            wrapped(ui, card.left_top() + Vec2::new(28.0, 66.0), card.width() - 56.0, &message, font(13.0), theme.text_dim);
            let b = Rect::from_min_size(Pos2::new(card.right() - 128.0, card.bottom() - 60.0), Vec2::new(100.0, 36.0));
            close = widgets::button(ui, b, Id::new("err-ok"), "OK", None, BtnStyle::Primary, &theme).clicked();
        });
        if close || bd { None } else { Some(Modal::Error { title: t, message }) }
    }

    fn settings_modal(&mut self, ctx: &egui::Context) -> Option<Modal> {
        let theme = self.theme.clone();
        let themes = self.themes.clone();
        let mut close = false;
        let mut new_theme: Option<String> = None;
        let mut s = self.settings.clone();
        let upd = self.updater.state();
        let mut check_now = false;
        let gl_error = self.gl_error.clone();
        let bd = modal_frame(ctx, self.modal_opened, Vec2::new(620.0, 710.0), &theme, |ui, card| {
            title(ui, card, "Settings", Some("Appearance, sounds and behaviour."), &theme);
            close = close_button(ui, card, &theme);
            let x0 = card.left() + 28.0;
            let w = card.width() - 56.0;
            let mut y = card.top() + 96.0;
            ui.painter().text(Pos2::new(x0, y), Align2::LEFT_TOP, "THEME", bold(11.0), theme.text_faint);
            y += 20.0;
            let cols = 4;
            let gap = 10.0;
            let tw = (w - gap * (cols as f32 - 1.0)) / cols as f32;
            let th = 76.0;
            for (k, t) in themes.iter().enumerate() {
                let r = Rect::from_min_size(Pos2::new(x0 + (k % cols) as f32 * (tw + gap), y + (k / cols) as f32 * (th + gap)), Vec2::new(tw, th));
                mark(format!("theme:{}", t.name), r);
                if theme_swatch(ui, r, t, t.name == theme.name, self.time).clicked() {
                    new_theme = Some(t.name.to_string());
                }
            }
            y += ((themes.len() + cols - 1) / cols) as f32 * (th + gap) + 12.0;

            let rows: [(&str, &str, u8); 6] = [
                ("Sound effects", "Plops, crunches and a little fanfare", 0),
                ("Shader effects", "GPU-rendered, anti-aliased segments (turn off on very old GPUs)", 1),
                ("Watch for changes", "Update the chart when files change on disk", 2),
                ("Check for updates", "Look for new releases on GitHub at launch", 3),
                ("Personalized sponsors", "Picked on this computer from disk categories; nothing about you is sent", 4),
                ("Anonymous sponsor stats", "Daily view totals and click counts, with no ID of any kind", 5),
            ];
            for (label, sub, k) in rows {
                let r = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(w, 44.0));
                let p = ui.painter();
                p.text(Pos2::new(r.left(), r.top() + 12.0), Align2::LEFT_CENTER, label, bold(13.5), theme.text);
                p.text(Pos2::new(r.left(), r.top() + 30.0), Align2::LEFT_CENTER, sub, font(11.5), theme.text_dim);
                let tr = Rect::from_center_size(Pos2::new(r.right() - 22.0, r.center().y), Vec2::new(44.0, 26.0));
                let v = match k {
                    0 => &mut s.sound,
                    1 => &mut s.shader_fx,
                    2 => &mut s.watch_fs,
                    3 => &mut s.auto_update,
                    4 => &mut s.personalized_sponsors,
                    _ => &mut s.sponsor_measurement,
                };
                widgets::toggle(ui, tr, Id::new(("set-toggle", k)), v, &theme);
                y += 46.0;
            }
            // Transparency: exactly what the sponsor feature knows, all local.
            let sig = self.sponsors.signals();
            let mut known: Vec<&str> = sig.interests.iter().map(|i| i.label()).collect();
            known.sort();
            let interests = if !s.personalized_sponsors { "off".to_string() } else if known.is_empty() { "none yet".to_string() } else { known.join(", ") };
            let local = format!("Known only on this device: {interests}  \u{00b7}  views waiting to be reported: {}", self.sponsors.pending_views());
            let local = widgets::truncate(ui.painter(), &local, &font(11.0), w);
            ui.painter().text(Pos2::new(x0, y + 4.0), Align2::LEFT_CENTER, local, font(11.0), theme.text_faint);
            y += 22.0;
            let p = ui.painter();
            p.text(Pos2::new(x0, y + 12.0), Align2::LEFT_CENTER, "Chart depth", bold(13.5), theme.text);
            p.text(Pos2::new(x0, y + 30.0), Align2::LEFT_CENTER, format!("{} rings — fewer is faster on old machines", s.rings), font(11.5), theme.text_dim);
            let sr = Rect::from_min_size(Pos2::new(x0 + w - 200.0, y + 8.0), Vec2::new(200.0, 28.0));
            widgets::stepper(ui, sr, Id::new("rings"), &mut s.rings, 3, 9, &theme);
            y += 50.0;

            let p = ui.painter();
            p.line_segment([Pos2::new(x0, y), Pos2::new(x0 + w, y)], Stroke::new(1.0, with_alpha(theme.stroke, 0.7)));
            let status = match &upd {
                UpState::Idle => "".to_string(),
                UpState::Checking => "Checking…".to_string(),
                UpState::UpToDate => "You're up to date".to_string(),
                UpState::Available(r) => format!("Version {} is available", r.version),
                UpState::Downloading => "Downloading update…".to_string(),
                UpState::Ready(v) => format!("{v} installed — restart to use it"),
                UpState::Failed(e) => format!("Update check failed: {e}"),
            };
            let mut line = format!("SquirrelDisk {}  ·  {}", env!("CARGO_PKG_VERSION"), status);
            if let Some(e) = &gl_error {
                line = format!("{line}  ·  CPU renderer ({e})");
            }
            let line = widgets::truncate(p, &line, &font(12.0), w - 170.0);
            p.text(Pos2::new(x0, y + 26.0), Align2::LEFT_CENTER, line, font(12.0), theme.text_dim);
            let b = Rect::from_min_size(Pos2::new(x0 + w - 160.0, y + 10.0), Vec2::new(160.0, 32.0));
            check_now = widgets::button(ui, b, Id::new("check-upd"), "Check for updates", Some(Icon::Refresh), BtnStyle::Subtle, &theme).clicked();
        });
        if check_now {
            self.updater.check(ctx.clone());
        }
        if s.sound && !self.settings.sound {
            self.settings.sound = true;
            self.sfx(Sfx::Plop);
        }
        let rings_changed = s.rings != self.settings.rings;
        if rings_changed {
            self.sfx(Sfx::Blip);
        }
        if let Some(n) = new_theme {
            s.theme = n.clone();
            self.theme = super::theme::by_name(&n);
            self.sfx(Sfx::Blip);
        }
        if !s.personalized_sponsors && self.settings.personalized_sponsors {
            self.sponsors.forget_interests();
        }
        let changed = serde_json::to_string(&s).ok() != serde_json::to_string(&self.settings).ok();
        self.settings = s;
        if changed {
            self.settings.save();
        }
        if close || bd { None } else { Some(Modal::Settings) }
    }

    fn ssh_modal(&mut self, ctx: &egui::Context, mut host: String, mut path: String) -> Option<Modal> {
        let theme = self.theme.clone();
        let mut close = false;
        let mut go = false;
        let mut hosts: Vec<String> = self.settings.ssh_history.clone();
        for h in &self.ssh_hosts {
            if !hosts.contains(h) {
                hosts.push(h.clone());
            }
        }
        let bd = modal_frame(ctx, self.modal_opened, Vec2::new(560.0, 420.0), &theme, |ui, card| {
            title(ui, card, "Scan a remote server", Some("Over SSH, using your keys and ~/.ssh/config."), &theme);
            close = close_button(ui, card, &theme);
            let x0 = card.left() + 28.0;
            let w = card.width() - 56.0;
            let mut y = card.top() + 96.0;
            let p = ui.painter();
            p.text(Pos2::new(x0, y), Align2::LEFT_TOP, "HOST", bold(11.0), theme.text_faint);
            y += 18.0;
            let r = widgets::text_input(ui, Rect::from_min_size(Pos2::new(x0, y), Vec2::new(w, 38.0)), Id::new("ssh-host"), &mut host, "user@server.example.com", &theme);
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            y += 46.0;
            // host chips
            let mut x = x0;
            for h in hosts.iter().take(8) {
                let wch = ui.painter().layout_no_wrap(h.clone(), font(12.0), Color32::WHITE).size().x + 20.0;
                if x + wch > x0 + w {
                    break;
                }
                let cr_ = Rect::from_min_size(Pos2::new(x, y), Vec2::new(wch, 26.0));
                let id = Id::new(("chip", h));
                let resp = ui.interact(cr_, id, Sense::click());
                let hh = ui.ctx().animate_bool_with_time(id, resp.hovered() || &host == h, 0.1);
                ui.painter().rect_filled(cr_, cr(6.0), lerp_color(theme.surface_hi, with_alpha(theme.accent, 0.5), hh));
                ui.painter().text(cr_.center(), Align2::CENTER_CENTER, h, font(12.0), theme.text);
                if resp.clicked() {
                    host = h.clone();
                }
                x += wch + 6.0;
            }
            if !hosts.is_empty() {
                y += 36.0;
            }
            ui.painter().text(Pos2::new(x0, y), Align2::LEFT_TOP, "PATH", bold(11.0), theme.text_faint);
            y += 18.0;
            widgets::text_input(ui, Rect::from_min_size(Pos2::new(x0, y), Vec2::new(w, 38.0)), Id::new("ssh-path"), &mut path, "/", &theme);
            y += 50.0;
            wrapped(
                ui,
                Pos2::new(x0, y),
                w,
                "SquirrelDisk installs a tiny agent in ~/.cache/squirreldisk on the server when possible, otherwise it falls back to find/du. Password prompts aren't supported: use key-based auth or ssh-agent.",
                font(11.5),
                theme.text_faint,
            );
            let b = Rect::from_min_size(Pos2::new(card.right() - 150.0, card.bottom() - 60.0), Vec2::new(122.0, 36.0));
            go |= widgets::button_ex(ui, b, Id::new("ssh-go"), "Connect", Some(Icon::Bolt), BtnStyle::Primary, &theme, !host.trim().is_empty()).clicked();
            let c = Rect::from_min_size(Pos2::new(card.right() - 260.0, card.bottom() - 60.0), Vec2::new(100.0, 36.0));
            close |= widgets::button(ui, c, Id::new("ssh-cancel"), "Cancel", None, BtnStyle::Ghost, &theme).clicked();
        });
        if go && !host.trim().is_empty() {
            let h = host.trim().to_string();
            self.settings.ssh_history.retain(|x| x != &h);
            self.settings.ssh_history.insert(0, h.clone());
            self.settings.ssh_history.truncate(10);
            self.settings.save();
            let p = if path.trim().is_empty() { "/".to_string() } else { path.trim().to_string() };
            self.modal = None;
            self.start_session(Target::Ssh { host: h, path: p });
            return None;
        }
        if close || bd { None } else { Some(Modal::Ssh { host, path }) }
    }

    fn rclone_modal(&mut self, ctx: &egui::Context, mut path: String) -> Option<Modal> {
        let theme = self.theme.clone();
        let remotes = self.rclone.lock().unwrap().clone();
        let missing = remotes.as_ref().map(|v| v.first().map(|s| s == "\u{0}").unwrap_or(false)).unwrap_or(false);
        let mut close = false;
        let mut go = false;
        let mut open_site = false;
        let bd = modal_frame(ctx, self.modal_opened, Vec2::new(560.0, 380.0), &theme, |ui, card| {
            title(ui, card, "Scan cloud storage", Some("S3, Google Drive, Dropbox, OneDrive, FTP, WebDAV… through rclone."), &theme);
            close = close_button(ui, card, &theme);
            let x0 = card.left() + 28.0;
            let w = card.width() - 56.0;
            let mut y = card.top() + 100.0;
            if missing || remotes.is_none() {
                let msg = if remotes.is_none() {
                    "Looking for rclone…"
                } else {
                    "SquirrelDisk talks to 70+ storage providers through rclone, a free command line tool. Install it, run `rclone config` once to add your accounts, then come back here."
                };
                wrapped(ui, Pos2::new(x0, y), w, msg, font(13.0), theme.text_dim);
                let b = Rect::from_min_size(Pos2::new(card.right() - 170.0, card.bottom() - 60.0), Vec2::new(142.0, 36.0));
                open_site = widgets::button(ui, b, Id::new("rc-site"), "Get rclone", Some(Icon::Download), BtnStyle::Primary, &theme).clicked();
                return;
            }
            let list = remotes.clone().unwrap_or_default();
            ui.painter().text(Pos2::new(x0, y), Align2::LEFT_TOP, "REMOTES", bold(11.0), theme.text_faint);
            y += 20.0;
            let mut x = x0;
            for r in list.iter() {
                let wch = ui.painter().layout_no_wrap(r.clone(), font(12.5), Color32::WHITE).size().x + 34.0;
                if x + wch > x0 + w {
                    x = x0;
                    y += 34.0;
                }
                let rr = Rect::from_min_size(Pos2::new(x, y), Vec2::new(wch, 28.0));
                let id = Id::new(("rc", r));
                let resp = ui.interact(rr, id, Sense::click());
                let hh = ui.ctx().animate_bool_with_time(id, resp.hovered() || path.starts_with(r.as_str()), 0.1);
                ui.painter().rect_filled(rr, cr(6.0), lerp_color(theme.surface_hi, with_alpha(theme.accent, 0.5), hh));
                widgets::draw_icon(ui.painter(), Icon::Cloud, Rect::from_center_size(Pos2::new(rr.left() + 14.0, rr.center().y), Vec2::splat(12.0)), theme.text_dim);
                ui.painter().text(Pos2::new(rr.left() + 26.0, rr.center().y), Align2::LEFT_CENTER, r, font(12.5), theme.text);
                if resp.clicked() {
                    path = r.clone();
                }
                x += wch + 6.0;
            }
            if list.is_empty() {
                ui.painter().text(Pos2::new(x0, y), Align2::LEFT_TOP, "No remotes yet — run `rclone config` in a terminal.", font(12.5), theme.warn);
            }
            y += 44.0;
            ui.painter().text(Pos2::new(x0, y), Align2::LEFT_TOP, "LOCATION", bold(11.0), theme.text_faint);
            y += 18.0;
            let r = widgets::text_input(ui, Rect::from_min_size(Pos2::new(x0, y), Vec2::new(w, 38.0)), Id::new("rc-path"), &mut path, "remote:bucket/folder", &theme);
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go = true;
            }
            let b = Rect::from_min_size(Pos2::new(card.right() - 150.0, card.bottom() - 60.0), Vec2::new(122.0, 36.0));
            go |= widgets::button_ex(ui, b, Id::new("rc-go"), "Scan", Some(Icon::Bolt), BtnStyle::Primary, &theme, path.contains(':')).clicked();
        });
        if open_site {
            super::app::os_open("https://rclone.org/install/", false);
        }
        if go && path.contains(':') {
            self.modal = None;
            self.start_session(Target::Rclone(path.trim().to_string()));
            return None;
        }
        if close || bd { None } else { Some(Modal::Rclone { path }) }
    }

    #[allow(clippy::too_many_arguments)]
    fn confirm_delete_modal(
        &mut self,
        ctx: &egui::Context,
        session: usize,
        items: Vec<super::app::DeleteItem>,
        mut mode: usize,
        mut backup_folder: String,
        mut backup_remote: String,
        mut ack: bool,
    ) -> Option<Modal> {
        let theme = self.theme.clone();
        let Some(s) = self.sessions.get(session) else { return None };
        let local = s.source().is_local();
        let has_rclone = self.rclone.lock().unwrap().as_ref().is_some_and(|v| v.first().map(|x| x != "\u{0}").unwrap_or(false));
        let ok_items: Vec<&super::app::DeleteItem> = items.iter().filter(|i| !i.verdict.is_forbidden()).collect();
        let total: u64 = ok_items.iter().map(|i| i.size).sum();
        let caution = items.iter().any(|i| matches!(i.verdict, Verdict::Caution(_)));
        let mut close = false;
        let mut confirm = false;
        let mut browse = false;
        let list_rows = items.len().min(5);
        let height = 330.0 + list_rows as f32 * 44.0 + if caution { 36.0 } else { 0.0 } + if mode >= 2 { 52.0 } else { 0.0 };
        let bd = modal_frame(ctx, self.modal_opened, Vec2::new(640.0, height), &theme, |ui, card| {
            let n = ok_items.len();
            let t = format!("Delete {} item{}?", n, if n == 1 { "" } else { "s" });
            title(ui, card, &t, Some(&format!("{} will be freed", fmt_size(total))), &theme);
            close = close_button(ui, card, &theme);
            let x0 = card.left() + 28.0;
            let w = card.width() - 56.0;
            let mut y = card.top() + 92.0;
            for it in items.iter().take(5) {
                let r = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(w, 40.0));
                let p = ui.painter();
                let (icon, col, note) = match &it.verdict {
                    Verdict::Ok => (Icon::Check, theme.ok, None),
                    Verdict::Caution(why) => (Icon::Warning, theme.warn, Some(why.clone())),
                    Verdict::Forbidden(why) => (Icon::Shield, theme.danger, Some(format!("protected, will be skipped: {why}"))),
                };
                p.rect_filled(r, cr(10.0), with_alpha(theme.bg_bottom, 0.45));
                widgets::draw_icon(p, icon, Rect::from_center_size(Pos2::new(r.left() + 18.0, r.center().y), Vec2::splat(14.0)), col);
                let lw = w - 150.0;
                p.text(Pos2::new(r.left() + 36.0, r.top() + 13.0), Align2::LEFT_CENTER, widgets::truncate(p, &it.name, &bold(12.5), lw), bold(12.5), if it.verdict.is_forbidden() { theme.text_faint } else { theme.text });
                let second = note.unwrap_or_else(|| it.path.clone());
                p.text(Pos2::new(r.left() + 36.0, r.top() + 28.0), Align2::LEFT_CENTER, widgets::truncate(p, &second, &font(11.0), lw), font(11.0), if it.verdict == Verdict::Ok { theme.text_faint } else { col });
                p.text(Pos2::new(r.right() - 12.0, r.center().y), Align2::RIGHT_CENTER, fmt_size(it.size), bold(12.5), theme.text_dim);
                y += 44.0;
            }
            if items.len() > 5 {
                ui.painter().text(Pos2::new(x0 + 4.0, y), Align2::LEFT_TOP, format!("and {} more…", items.len() - 5), font(12.0), theme.text_faint);
                y += 20.0;
            }
            y += 8.0;
            // modes
            let gap = 10.0;
            let cw = (w - gap) / 2.0;
            let modes: [(&str, &str, Icon, bool); 4] = [
                ("Move to Trash", "Recoverable from the system trash", Icon::Trash, local),
                ("Delete permanently", "Frees space now, no undo", Icon::Bolt, true),
                ("Back up, then delete", "Copy to a folder (e.g. external disk)", Icon::External, true),
                ("Cloud backup, then delete", if has_rclone { "Upload with rclone first" } else { "Needs rclone" }, Icon::Cloud, local && has_rclone),
            ];
            for (k, (tt, sub, icon, en)) in modes.iter().enumerate() {
                let r = Rect::from_min_size(Pos2::new(x0 + (k % 2) as f32 * (cw + gap), y + (k / 2) as f32 * 62.0), Vec2::new(cw, 54.0));
                mark(format!("mode-{k}"), r);
                if widgets::choice_card(ui, r, Id::new(("mode", k)), mode == k, tt, sub, *icon, &theme, *en).clicked() {
                    mode = k;
                }
            }
            y += 128.0;
            if mode == 2 {
                let r = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(w - 110.0, 38.0));
                widgets::text_input(ui, r, Id::new("bk-folder"), &mut backup_folder, "Backup folder, e.g. /Volumes/External", &theme);
                let b = Rect::from_min_size(Pos2::new(x0 + w - 100.0, y + 2.0), Vec2::new(100.0, 34.0));
                browse = widgets::button(ui, b, Id::new("bk-browse"), "Browse…", Some(Icon::Folder), BtnStyle::Subtle, &theme).clicked();
                y += 52.0;
            } else if mode == 3 {
                let r = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(w, 38.0));
                widgets::text_input(ui, r, Id::new("bk-remote"), &mut backup_remote, "remote:bucket/backups", &theme);
                y += 52.0;
            }
            if caution {
                let r = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(w, 28.0));
                widgets::checkbox(ui, r, Id::new("ack"), &mut ack, "I know some items are app or system data and want to delete them anyway", &theme);
            }
            let backup_ok = match mode {
                2 => !backup_folder.trim().is_empty() && std::path::Path::new(backup_folder.trim()).is_dir(),
                3 => backup_remote.contains(':'),
                _ => true,
            };
            let enabled = !ok_items.is_empty() && backup_ok && (!caution || ack);
            let label = match mode {
                0 => "Move to Trash",
                2 | 3 => "Back up & delete",
                _ => "Delete forever",
            };
            let b = Rect::from_min_size(Pos2::new(card.right() - 196.0, card.bottom() - 60.0), Vec2::new(168.0, 38.0));
            mark("del-go", b);
            confirm = widgets::button_ex(ui, b, Id::new("del-go"), label, Some(Icon::Trash), BtnStyle::Danger, &theme, enabled).clicked();
            let c = Rect::from_min_size(Pos2::new(card.right() - 306.0, card.bottom() - 60.0), Vec2::new(100.0, 38.0));
            close |= widgets::button(ui, c, Id::new("del-cancel"), "Cancel", None, BtnStyle::Ghost, &theme).clicked();
            if mode == 2 && !backup_folder.trim().is_empty() && !backup_ok {
                ui.painter().text(Pos2::new(x0, card.bottom() - 40.0), Align2::LEFT_CENTER, "Folder not found", font(12.0), theme.danger);
            }
        });
        if browse {
            if let Some(p) = rfd::FileDialog::new().set_title("Choose where to put the backup").pick_folder() {
                backup_folder = p.to_string_lossy().into_owned();
            }
        }
        if confirm {
            let m = match mode {
                0 => Mode::Trash,
                2 => {
                    self.settings.backup_folder = Some(backup_folder.trim().to_string());
                    Mode::BackupFolder(backup_folder.trim().into())
                }
                3 => {
                    self.settings.backup_remote = Some(backup_remote.trim().to_string());
                    Mode::BackupRclone(backup_remote.trim().to_string())
                }
                _ => Mode::Permanent,
            };
            self.settings.save();
            self.start_delete(session, items, m);
            return None; // start_delete opened the progress modal
        }
        if close || bd {
            None
        } else {
            Some(Modal::ConfirmDelete { session, items, mode, backup_folder, backup_remote, ack })
        }
    }

    fn deleting_modal(&mut self, ctx: &egui::Context, session: usize, mut finished_at: Option<Instant>) -> Option<Modal> {
        let theme = self.theme.clone();
        let Some(job) = self.sessions.get(session).and_then(|s| s.delete.clone()) else { return None };
        let finished = job.finished.load(Ordering::Relaxed);
        if finished && finished_at.is_none() {
            finished_at = Some(Instant::now());
            let errs = job.errors.lock().unwrap().len();
            let freed = job.freed.load(Ordering::Relaxed);
            if freed > 0 {
                self.sfx(Sfx::Success);
                let colors: Vec<Color32> = theme.wheel.iter().map(|c| Color32::from_rgb(c[0], c[1], c[2])).collect();
                self.particles.confetti(ctx.content_rect(), &colors);
            } else if errs > 0 {
                self.sfx(Sfx::Error);
            }
        }
        let frac = job.fraction();
        let freed = job.freed.load(Ordering::Relaxed);
        let total = job.total_bytes.load(Ordering::Relaxed);
        let errors = job.errors.lock().unwrap().clone();
        let phase = job.phase.lock().unwrap().clone();
        let current = job.current.lock().unwrap().clone();
        let backup = job.backup_location.lock().unwrap().clone();
        let items_done = job.done_items.load(Ordering::Relaxed);
        let items_total = job.total_items.load(Ordering::Relaxed);
        let mut close = false;
        let mut cancel = false;
        let h = 380.0 + if errors.is_empty() { 0.0 } else { 90.0 };
        let time = self.time;
        modal_frame(ctx, self.modal_opened, Vec2::new(520.0, h), &theme, |ui, card| {
            let p = ui.painter();
            let c = Pos2::new(card.center().x, card.top() + 120.0);
            let disp = ui.ctx().animate_value_with_time(Id::new("del-frac"), if finished { 1.0 } else { frac }, 0.25);
            // glowing ring
            widgets::glow(p, c, 110.0, with_alpha(if finished { theme.ok } else { theme.danger }, 0.18));
            p.circle_stroke(c, 70.0, Stroke::new(10.0, with_alpha(theme.bg_bottom, 0.8)));
            let n = 120;
            let seg = ((disp * n as f32) as usize).max(1);
            for i in 0..seg {
                let a0 = -TAU / 4.0 + TAU * i as f32 / n as f32;
                let a1 = -TAU / 4.0 + TAU * (i + 1) as f32 / n as f32;
                let col = lerp_color(theme.accent2, if finished { theme.ok } else { theme.danger }, i as f32 / n as f32);
                p.line_segment([c + Vec2::angled(a0) * 70.0, c + Vec2::angled(a1) * 70.0], Stroke::new(10.0, col));
            }
            if !finished {
                let a = time as f32 * 3.0;
                p.circle_filled(c + Vec2::angled(a) * 86.0, 3.0, with_alpha(theme.danger, 0.8));
            }
            if finished {
                widgets::draw_icon(p, Icon::Check, Rect::from_center_size(c - Vec2::new(0.0, 14.0), Vec2::splat(30.0)), theme.ok);
                p.text(c + Vec2::new(0.0, 22.0), Align2::CENTER_CENTER, "Done", display(18.0), theme.text);
            } else {
                p.text(c - Vec2::new(0.0, 6.0), Align2::CENTER_CENTER, format!("{:.0}%", disp * 100.0), display(30.0), theme.text);
                p.text(c + Vec2::new(0.0, 22.0), Align2::CENTER_CENTER, format!("{items_done} / {items_total}"), font(12.0), theme.text_dim);
            }
            let mut y = card.top() + 222.0;
            let headline = if finished {
                format!("Freed {}", fmt_size(freed))
            } else {
                format!("{phase}… {} freed of {}", fmt_size(freed), fmt_size(total / if backup.is_some() { 2 } else { 1 }))
            };
            p.text(Pos2::new(card.center().x, y), Align2::CENTER_CENTER, headline, bold(15.0), theme.text);
            y += 24.0;
            let sub = if finished {
                backup.map(|b| format!("Backup saved to {b}")).unwrap_or_else(|| "The squirrel is pleased.".into())
            } else {
                current.clone()
            };
            let sub = widgets::truncate(p, &sub, &font(11.5), card.width() - 60.0);
            p.text(Pos2::new(card.center().x, y), Align2::CENTER_CENTER, sub, font(11.5), theme.text_faint);
            y += 24.0;
            if !errors.is_empty() {
                let r = Rect::from_min_size(Pos2::new(card.left() + 28.0, y), Vec2::new(card.width() - 56.0, 80.0));
                p.rect_filled(r, cr(10.0), with_alpha(theme.danger, 0.12));
                p.text(r.left_top() + Vec2::new(12.0, 14.0), Align2::LEFT_CENTER, format!("{} problem{}", errors.len(), if errors.len() == 1 { "" } else { "s" }), bold(12.0), theme.danger);
                for (k, e) in errors.iter().take(3).enumerate() {
                    let e = widgets::truncate(p, e, &font(11.0), r.width() - 24.0);
                    p.text(r.left_top() + Vec2::new(12.0, 32.0 + k as f32 * 15.0), Align2::LEFT_CENTER, e, font(11.0), theme.text_dim);
                }
            }
            let b = Rect::from_min_size(Pos2::new(card.center().x - 60.0, card.bottom() - 58.0), Vec2::new(120.0, 38.0));
            if finished {
                close = widgets::button(ui, b, Id::new("del-close"), "Close", None, BtnStyle::Primary, &theme).clicked();
            } else {
                cancel = widgets::button(ui, b, Id::new("del-stop"), "Stop", Some(Icon::Close), BtnStyle::Subtle, &theme).clicked();
            }
        });
        if cancel {
            job.cancel.store(true, Ordering::Relaxed);
        }
        if close {
            if let Some(s) = self.sessions.get_mut(session) {
                s.delete = None;
            }
            return None;
        }
        Some(Modal::Deleting { session, finished_at })
    }
}

/// Mini preview of a theme: its background with a tiny sunburst.
fn theme_swatch(ui: &mut Ui, r: Rect, t: &Theme, selected: bool, time: f64) -> egui::Response {
    let id = Id::new(("swatch", t.name));
    let resp = ui.interact(r, id, Sense::click());
    let h = ui.ctx().animate_bool_with_time(id.with("h"), resp.hovered(), 0.12);
    let s = ui.ctx().animate_bool_with_time(id.with("s"), selected, 0.15);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let p = ui.painter();
    let rr = r.translate(Vec2::new(0.0, -2.0 * h));
    p.rect_filled(rr, cr(12.0), lerp_color(t.bg_top, t.bg_bottom, 0.5));
    let c = Pos2::new(rr.left() + 32.0, rr.top() + 30.0);
    let spin = if h > 0.0 { (time as f32 * 0.8) * h } else { 0.0 };
    let n = 10;
    for ring in 0..2 {
        let (r0, r1) = (7.0 + ring as f32 * 7.0, 13.0 + ring as f32 * 7.0);
        for i in 0..n {
            let a0 = spin + TAU * i as f32 / n as f32 + 0.03;
            let a1 = spin + TAU * (i + 1) as f32 / n as f32 - 0.03;
            let col = t.segment_color(i as f32 / n as f32, ring + 1, false);
            let mut pts = Vec::new();
            for k in 0..=6 {
                let a = a0 + (a1 - a0) * k as f32 / 6.0;
                pts.push(c + Vec2::angled(a) * r1);
            }
            for k in (0..=6).rev() {
                let a = a0 + (a1 - a0) * k as f32 / 6.0;
                pts.push(c + Vec2::angled(a) * r0);
            }
            // non-convex: draw as two triangles fan per step
            for k in 0..6 {
                let q = vec![pts[k], pts[k + 1], pts[13 - k - 1], pts[13 - k]];
                p.add(Shape::convex_polygon(q, col, Stroke::NONE));
            }
        }
    }
    p.circle_filled(c, 6.5, t.surface);
    // fake UI lines
    for k in 0..3 {
        let y = rr.top() + 18.0 + k as f32 * 10.0;
        p.rect_filled(Rect::from_min_size(Pos2::new(rr.left() + 62.0, y), Vec2::new(rr.width() - 76.0 - k as f32 * 8.0, 4.0)), cr(2.0), with_alpha(t.text_dim, 0.5));
    }
    p.text(Pos2::new(rr.left() + 10.0, rr.bottom() - 13.0), Align2::LEFT_CENTER, t.name, bold(11.5), t.text);
    p.rect_stroke(rr, cr(12.0), Stroke::new(1.0 + 1.5 * s, lerp_color(with_alpha(t.stroke, 0.8), t.accent, s.max(h * 0.5))), egui::StrokeKind::Inside);
    resp
}
