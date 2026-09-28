//! Cloud storage dialogs: install rclone, add / edit / remove accounts,
//! pick what to scan.

use super::app::{refresh_rclone, App, Modal, RcloneState, Target};
use super::modals::{close_button, mark, modal_frame, title, wrapped};
use super::theme::{lerp_color, with_alpha, Theme};
use super::widgets::{self, bold, cr, font, BtnStyle, Icon};
use crate::i18n::{tr, trf};
use crate::rclone::{self, Remote, Task};
use crate::tree::fmt_size;
use eframe::egui::{self, Align2, Id, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    pub secret: bool,
    pub optional: bool,
    pub default: &'static str,
}

const fn f(key: &'static str, label: &'static str, hint: &'static str) -> Field {
    Field { key, label, hint, secret: false, optional: false, default: "" }
}
const fn opt(key: &'static str, label: &'static str, hint: &'static str) -> Field {
    Field { key, label, hint, secret: false, optional: true, default: "" }
}
const fn secret(key: &'static str, label: &'static str, hint: &'static str, optional: bool) -> Field {
    Field { key, label, hint, secret: true, optional, default: "" }
}

pub struct Provider {
    pub kind: &'static str,
    pub label: &'static str,
    /// Suggested remote name.
    pub short: &'static str,
    /// Sign-in happens in the browser (OAuth).
    pub oauth: bool,
    pub icon: Icon,
    pub fields: &'static [Field],
}

pub static PROVIDERS: &[Provider] = &[
    Provider { kind: "drive", label: "Google Drive", short: "gdrive", oauth: true, icon: Icon::Cloud, fields: &[] },
    Provider { kind: "dropbox", label: "Dropbox", short: "dropbox", oauth: true, icon: Icon::Cloud, fields: &[] },
    Provider { kind: "onedrive", label: "OneDrive", short: "onedrive", oauth: true, icon: Icon::Cloud, fields: &[] },
    Provider { kind: "box", label: "Box", short: "box", oauth: true, icon: Icon::Cloud, fields: &[] },
    Provider { kind: "pcloud", label: "pCloud", short: "pcloud", oauth: true, icon: Icon::Cloud, fields: &[] },
    Provider {
        kind: "s3",
        label: "S3 & compatible",
        short: "s3",
        oauth: false,
        icon: Icon::Disk,
        fields: &[
            Field { key: "provider", label: "Provider", hint: "AWS, Cloudflare, Wasabi, Minio, Other", secret: false, optional: false, default: "AWS" },
            f("access_key_id", "Access key ID", ""),
            secret("secret_access_key", "Secret access key", "", false),
            opt("region", "Region", "us-east-1"),
            opt("endpoint", "Endpoint", "Only for non-AWS providers"),
        ],
    },
    Provider {
        kind: "b2",
        label: "Backblaze B2",
        short: "b2",
        oauth: false,
        icon: Icon::Disk,
        fields: &[f("account", "Application key ID", ""), secret("key", "Application key", "", false)],
    },
    Provider {
        kind: "sftp",
        label: "SFTP",
        short: "sftp",
        oauth: false,
        icon: Icon::Server,
        fields: &[
            f("host", "Host", "example.com"),
            opt("user", "User", ""),
            secret("pass", "Password", "Empty = SSH key or agent", true),
            opt("port", "Port", "22"),
            opt("key_file", "Key file", "~/.ssh/id_ed25519"),
        ],
    },
    Provider {
        kind: "ftp",
        label: "FTP",
        short: "ftp",
        oauth: false,
        icon: Icon::Server,
        fields: &[f("host", "Host", "ftp.example.com"), opt("user", "User", "anonymous"), secret("pass", "Password", "", true), opt("port", "Port", "21")],
    },
    Provider {
        kind: "webdav",
        label: "WebDAV / Nextcloud",
        short: "webdav",
        oauth: false,
        icon: Icon::Server,
        fields: &[
            f("url", "URL", "https://cloud.example.com/remote.php/dav/files/me"),
            opt("vendor", "Vendor", "nextcloud, owncloud, sharepoint, other"),
            opt("user", "User", ""),
            secret("pass", "Password", "", true),
        ],
    },
    Provider { kind: "mega", label: "MEGA", short: "mega", oauth: false, icon: Icon::Cloud, fields: &[f("user", "Email", ""), secret("pass", "Password", "", false)] },
];

pub fn provider(kind: &str) -> Option<usize> {
    PROVIDERS.iter().position(|p| p.kind == kind)
}

fn kind_label(kind: &str) -> String {
    provider(kind).map(|i| PROVIDERS[i].label.to_string()).unwrap_or_else(|| kind.to_string())
}

#[derive(Clone, Copy, PartialEq)]
enum Pending {
    Save,
    Test,
    Reconnect,
}

/// State of the add / edit account dialog.
pub struct AccountForm {
    /// Name of the remote being edited (None while adding a new one).
    original: Option<String>,
    /// Chosen provider (None: still on the provider grid).
    provider: Option<usize>,
    kind: String,
    name: String,
    values: Vec<String>,
    /// Values as loaded, to send only what changed.
    loaded: Vec<String>,
    /// Full config of the remote being edited (rename = copy + delete).
    orig_params: Vec<(String, String)>,
    task: Option<(Arc<Task>, Pending)>,
    error: Option<String>,
    /// Location typed in the Cloud storage dialog, restored on the way back.
    back_path: String,
}

impl AccountForm {
    pub fn new(back_path: String) -> Self {
        AccountForm { original: None, provider: None, kind: String::new(), name: String::new(), values: vec![], loaded: vec![], orig_params: vec![], task: None, error: None, back_path }
    }

    pub fn edit(r: &Remote, back_path: String) -> Self {
        let p = provider(&r.kind);
        let values: Vec<String> = p
            .map(|i| PROVIDERS[i].fields.iter().map(|f| if f.secret { String::new() } else { r.param(f.key).unwrap_or("").to_string() }).collect())
            .unwrap_or_default();
        AccountForm {
            original: Some(r.name.clone()),
            provider: p,
            kind: r.kind.clone(),
            name: r.name.clone(),
            loaded: values.clone(),
            values,
            orig_params: r.params.clone(),
            task: None,
            error: None,
            back_path,
        }
    }

    /// Straight to the form of one provider (debug screenshots).
    pub fn with_provider(kind: &str, existing: &[Remote]) -> Self {
        let mut f = Self::new(String::new());
        if let Some(i) = provider(kind) {
            f.pick(i, existing);
        }
        f
    }

    fn pick(&mut self, i: usize, existing: &[Remote]) {
        let p = &PROVIDERS[i];
        self.provider = Some(i);
        self.kind = p.kind.to_string();
        self.values = p.fields.iter().map(|f| f.default.to_string()).collect();
        self.loaded = vec![String::new(); p.fields.len()];
        let mut name = p.short.to_string();
        let mut n = 2;
        while existing.iter().any(|r| r.name.eq_ignore_ascii_case(&name)) {
            name = format!("{}{n}", p.short);
            n += 1;
        }
        self.name = name;
        self.error = None;
    }

    fn ready(&self) -> bool {
        let Some(i) = self.provider else { return self.original.is_some() && rclone::valid_name(self.name.trim()) };
        let editing = self.original.is_some();
        rclone::valid_name(self.name.trim())
            && PROVIDERS[i].fields.iter().zip(&self.values).all(|(f, v)| f.optional || (f.secret && editing) || !v.trim().is_empty())
    }

    /// rclone commands that save the form.
    fn save_steps(&self) -> Vec<Vec<String>> {
        let name = self.name.trim().to_string();
        let mut steps = Vec::new();
        let fields = self.provider.map(|i| PROVIDERS[i].fields).unwrap_or(&[]);
        match &self.original {
            None => {
                let mut a = vec!["config".into(), "create".into(), name.clone(), self.kind.clone()];
                for (f, v) in fields.iter().zip(&self.values) {
                    if !v.trim().is_empty() {
                        a.push(format!("{}={}", f.key, v.trim()));
                    }
                }
                if self.provider.is_some_and(|i| PROVIDERS[i].oauth) {
                    a.push("config_is_local=true".into());
                }
                a.push("--obscure".into());
                steps.push(a);
            }
            Some(old) => {
                // rclone has no rename: copy the config as it is, then drop the old one
                let renamed = old != &name;
                if renamed {
                    let mut a = vec!["config".into(), "create".into(), name.clone(), self.kind.clone()];
                    a.extend(self.orig_params.iter().map(|(k, v)| format!("{k}={v}")));
                    a.push("--no-obscure".into());
                    a.push("--non-interactive".into());
                    steps.push(a);
                }
                let mut a = vec!["config".into(), "update".into(), name.clone()];
                for ((f, v), was) in fields.iter().zip(&self.values).zip(&self.loaded) {
                    let changed = if f.secret { !v.is_empty() } else { v.trim() != was.trim() };
                    if changed {
                        a.push(format!("{}={}", f.key, v.trim()));
                    }
                }
                if a.len() > 3 {
                    a.push("--obscure".into());
                    // don't start the browser sign-in again for a plain edit
                    a.push("config_refresh_token=false".into());
                    steps.push(a);
                }
                if renamed {
                    steps.push(vec!["config".into(), "delete".into(), old.clone()]);
                }
            }
        }
        steps
    }
}

impl AccountForm {
    /// Stops what's running; a sign-in abandoned half way leaves nothing behind.
    fn cancel(&mut self, state: &Arc<std::sync::Mutex<RcloneState>>, ctx: &egui::Context) {
        if let Some((t, what)) = self.task.take() {
            t.cancel();
            if what == Pending::Save && self.original.is_none() {
                forget(self.name.trim().to_string(), state, ctx);
            }
        }
    }
}

/// Removes a remote whose creation didn't complete, then lists again.
fn forget(name: String, state: &Arc<std::sync::Mutex<RcloneState>>, ctx: &egui::Context) {
    let (state, ctx) = (state.clone(), ctx.clone());
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let _ = rclone::command().args(["config", "delete", &name]).stdin(std::process::Stdio::null()).status();
        refresh_rclone(&state, &ctx);
    });
}

fn test_args(name: &str) -> Vec<String> {
    ["lsd", &format!("{name}:"), "--max-depth", "1", "--contimeout", "15s", "--timeout", "30s", "--retries", "1", "--low-level-retries", "1"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Password-style input (dots).
fn secret_input(ui: &mut Ui, rect: Rect, id: Id, value: &mut String, hint: &str, theme: &Theme) {
    let focused = ui.memory(|m| m.has_focus(id));
    let t = ui.ctx().animate_bool_with_time(id.with("f"), focused, 0.15);
    {
        let p = ui.painter();
        p.rect_filled(rect, cr(6.0), lerp_color(with_alpha(theme.bg_bottom, 0.6), with_alpha(theme.bg_bottom, 0.9), t));
        p.rect_stroke(rect, cr(6.0), Stroke::new(1.0, lerp_color(theme.stroke, theme.accent, t)), egui::StrokeKind::Inside);
    }
    let edit = egui::TextEdit::singleline(value)
        .id(id)
        .password(true)
        .frame(egui::Frame::NONE)
        .font(font(14.0))
        .text_color(theme.text)
        .hint_text(egui::RichText::new(hint).color(theme.text_faint).font(font(14.0)))
        .desired_width(rect.width() - 24.0)
        .vertical_align(egui::Align::Center);
    ui.put(rect.shrink2(Vec2::new(12.0, 0.0)), edit);
}

/// Small spinning arc.
fn spinner(ui: &Ui, c: Pos2, r: f32, color: egui::Color32, time: f64) {
    let a0 = (time * 5.0) as f32;
    let pts: Vec<Pos2> = (0..=24).map(|k| a0 + k as f32 / 24.0 * 4.2).map(|a| c + Vec2::angled(a) * r).collect();
    ui.painter().add(egui::Shape::line(pts, Stroke::new(2.0, color)));
    ui.ctx().request_repaint();
}

impl App {
    pub(super) fn cloud_modal(&mut self, ctx: &egui::Context, mut path: String, mut confirm_remove: Option<String>) -> Option<Modal> {
        let theme = self.theme.clone();
        let state = self.rclone.lock().unwrap().clone();
        let install = self.rclone_install.clone();
        // the download finished: look again
        if let Some(t) = &install {
            if let Some(Ok(())) = t.finished() {
                self.rclone_install = None;
                *self.rclone.lock().unwrap() = RcloneState::Looking;
                refresh_rclone(&self.rclone, ctx);
            }
        }
        let time = self.time;
        let mut close = false;
        let mut go = false;
        let mut start_install = false;
        let mut cancel_install = false;
        let mut other_ways = false;
        let mut terminal = false;
        let mut refresh = false;
        let mut add = false;
        let mut edit: Option<Remote> = None;
        let mut remove: Option<String> = None;
        let remotes = match &state {
            RcloneState::Ready(v) => v.clone(),
            _ => vec![],
        };
        let row_h = 46.0;
        let visible = remotes.len().clamp(1, 5) as f32;
        let list_h = if remotes.is_empty() { 96.0 } else { visible * (row_h + 6.0) - 6.0 };
        let height = match state {
            RcloneState::Ready(_) => 100.0 + 26.0 + list_h + 24.0 + 18.0 + 38.0 + 90.0,
            _ => 360.0,
        };
        let bd = modal_frame(ctx, self.modal_opened, Vec2::new(580.0, height), &theme, |ui, card| {
            title(ui, card, tr("Cloud storage"), Some(tr("Google Drive, Dropbox, OneDrive, S3, SFTP, WebDAV… through rclone.")), &theme);
            close = close_button(ui, card, &theme);
            let x0 = card.left() + 28.0;
            let w = card.width() - 56.0;
            let mut y = card.top() + 100.0;
            let foot_y = card.bottom() - 60.0;
            match &state {
                RcloneState::Looking => {
                    spinner(ui, Pos2::new(x0 + 8.0, y + 9.0), 7.0, theme.accent, time);
                    ui.painter().text(Pos2::new(x0 + 26.0, y + 9.0), Align2::LEFT_CENTER, tr("Looking for rclone…"), font(13.0), theme.text_dim);
                }
                RcloneState::Missing => {
                    y += wrapped(
                        ui,
                        Pos2::new(x0, y),
                        w,
                        tr("SquirrelDisk reaches 70+ storage providers through rclone, a free open-source tool. Install it with one click: it goes into SquirrelDisk's own folder and needs no admin rights."),
                        font(13.0),
                        theme.text_dim,
                    ) + 22.0;
                    match install.as_ref().map(|t| (t.clone(), t.finished())) {
                        Some((t, None)) => {
                            let got = t.got.load(Ordering::Relaxed);
                            let total = t.total.load(Ordering::Relaxed);
                            let frac = (total > 0).then(|| got as f32 / total as f32);
                            let label = if total > 0 {
                                trf("Downloading rclone… {done} of {total}", &[("done", &fmt_size(got)), ("total", &fmt_size(total))])
                            } else {
                                tr("Downloading rclone…").to_string()
                            };
                            ui.painter().text(Pos2::new(x0, y), Align2::LEFT_TOP, label, font(12.5), theme.text);
                            widgets::progress_bar(ui.painter(), Rect::from_min_size(Pos2::new(x0, y + 24.0), Vec2::new(w, 8.0)), frac, &theme, time, None);
                            ui.ctx().request_repaint();
                            let c = Rect::from_min_size(Pos2::new(card.right() - 128.0, foot_y), Vec2::new(100.0, 36.0));
                            cancel_install = widgets::button(ui, c, Id::new("rc-cancel"), tr("Cancel"), None, BtnStyle::Ghost, &theme).clicked();
                        }
                        other => {
                            if let Some((_, Some(Err(e)))) = other {
                                if e != "cancelled" {
                                    wrapped(ui, Pos2::new(x0, y), w, &trf("Couldn't install rclone: {error}", &[("error", &e)]), font(12.5), theme.danger);
                                }
                            }
                            let b = Rect::from_min_size(Pos2::new(card.right() - 190.0, foot_y), Vec2::new(162.0, 36.0));
                            mark("rc-install", b);
                            start_install = widgets::button(ui, b, Id::new("rc-install"), tr("Install rclone"), Some(Icon::Download), BtnStyle::Primary, &theme).clicked();
                            let o = Rect::from_min_size(Pos2::new(x0 - 8.0, foot_y), Vec2::new(200.0, 36.0));
                            other_ways = widgets::button(ui, o, Id::new("rc-site"), tr("Other ways to install"), Some(Icon::LinkOut), BtnStyle::Ghost, &theme).clicked();
                        }
                    }
                }
                RcloneState::Ready(_) => {
                    ui.painter().text(Pos2::new(x0, y + 4.0), Align2::LEFT_TOP, tr("ACCOUNTS"), bold(11.0), theme.text_faint);
                    let rb = Rect::from_center_size(Pos2::new(x0 + w - 14.0, y + 10.0), Vec2::splat(26.0));
                    refresh = widgets::icon_button(ui, rb, Id::new("rc-refresh"), Icon::Refresh, &theme, true).on_hover_text(tr("Refresh")).clicked();
                    if !remotes.is_empty() {
                        let ab = Rect::from_min_size(Pos2::new(rb.left() - 150.0, y - 4.0), Vec2::new(142.0, 28.0));
                        mark("rc-add", ab);
                        add = widgets::button(ui, ab, Id::new("rc-add"), tr("Add account"), Some(Icon::Plus), BtnStyle::Subtle, &theme).clicked();
                    }
                    y += 30.0;
                    let list = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(w, list_h));
                    if remotes.is_empty() {
                        let p = ui.painter();
                        p.rect_stroke(list, cr(10.0), Stroke::new(1.0, theme.stroke), egui::StrokeKind::Inside);
                        p.text(Pos2::new(list.center().x, list.top() + 26.0), Align2::CENTER_CENTER, tr("No accounts yet"), bold(13.5), theme.text);
                        let ab = Rect::from_center_size(Pos2::new(list.center().x, list.top() + 62.0), Vec2::new(170.0, 34.0));
                        mark("rc-add", ab);
                        add = widgets::button(ui, ab, Id::new("rc-add"), tr("Add account"), Some(Icon::Plus), BtnStyle::Primary, &theme).clicked();
                    } else {
                        // scrollable when there are many accounts
                        let content = remotes.len() as f32 * (row_h + 6.0) - 6.0;
                        let sid = Id::new("rc-scroll");
                        let mut scroll: f32 = ui.data(|d| d.get_temp(sid)).unwrap_or(0.0);
                        if ui.rect_contains_pointer(list) {
                            scroll -= ui.input(|i| i.smooth_scroll_delta.y);
                        }
                        scroll = scroll.clamp(0.0, (content - list_h).max(0.0));
                        ui.data_mut(|d| d.insert_temp(sid, scroll));
                        ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
                            ui.set_clip_rect(list.expand(1.0).intersect(ui.clip_rect()));
                            for (k, r) in remotes.iter().enumerate() {
                                let rr = Rect::from_min_size(Pos2::new(x0, list.top() + k as f32 * (row_h + 6.0) - scroll), Vec2::new(w, row_h));
                                if !rr.intersects(list) {
                                    continue;
                                }
                                let target = format!("{}:", r.name);
                                let selected = path == target || path.starts_with(&target);
                                let id = Id::new(("rc-row", &r.name));
                                let resp = ui.interact(rr, id, Sense::click());
                                let h = ui.ctx().animate_bool_with_time(id, resp.hovered(), 0.1);
                                let s = ui.ctx().animate_bool_with_time(id.with("s"), selected, 0.12);
                                let confirming = confirm_remove.as_deref() == Some(r.name.as_str());
                                let fill = if confirming {
                                    with_alpha(theme.danger, 0.16)
                                } else {
                                    lerp_color(lerp_color(theme.surface, theme.surface_hi, h), with_alpha(theme.accent, 0.22), s)
                                };
                                let icon = provider(&r.kind).map(|i| PROVIDERS[i].icon).unwrap_or(Icon::Cloud);
                                {
                                    let p = ui.painter();
                                    p.rect_filled(rr, cr(8.0), fill);
                                    p.rect_stroke(rr, cr(8.0), Stroke::new(1.0, lerp_color(theme.stroke, theme.accent, s)), egui::StrokeKind::Inside);
                                    widgets::draw_icon(p, icon, Rect::from_center_size(Pos2::new(rr.left() + 22.0, rr.center().y), Vec2::splat(16.0)), lerp_color(theme.text_dim, theme.accent, s));
                                    if confirming {
                                        p.text(Pos2::new(rr.left() + 44.0, rr.center().y), Align2::LEFT_CENTER, trf("Remove {name} from rclone?", &[("name", &r.name)]), bold(13.0), theme.text);
                                    } else {
                                        p.text(Pos2::new(rr.left() + 44.0, rr.center().y - 8.0), Align2::LEFT_CENTER, &r.name, bold(13.5), theme.text);
                                        p.text(Pos2::new(rr.left() + 44.0, rr.center().y + 9.0), Align2::LEFT_CENTER, kind_label(&r.kind), font(11.5), theme.text_dim);
                                    }
                                }
                                if confirming {
                                    let yes = Rect::from_min_size(Pos2::new(rr.right() - 184.0, rr.top() + 8.0), Vec2::new(88.0, 30.0));
                                    let no = Rect::from_min_size(Pos2::new(rr.right() - 92.0, rr.top() + 8.0), Vec2::new(84.0, 30.0));
                                    if widgets::button(ui, yes, id.with("yes"), tr("Remove"), None, BtnStyle::Danger, &theme).clicked() {
                                        remove = Some(r.name.clone());
                                    }
                                    if widgets::button(ui, no, id.with("no"), tr("Cancel"), None, BtnStyle::Ghost, &theme).clicked() {
                                        confirm_remove = None;
                                    }
                                    continue;
                                }
                                let mut consumed = false;
                                if h.max(s) > 0.01 {
                                    let eb = Rect::from_center_size(Pos2::new(rr.right() - 56.0, rr.center().y), Vec2::splat(28.0));
                                    let tb = Rect::from_center_size(Pos2::new(rr.right() - 22.0, rr.center().y), Vec2::splat(28.0));
                                    if widgets::icon_button(ui, eb, id.with("edit"), Icon::Pencil, &theme, true).on_hover_text(tr("Edit")).clicked() {
                                        edit = Some(r.clone());
                                        consumed = true;
                                    }
                                    if widgets::icon_button(ui, tb, id.with("del"), Icon::Trash, &theme, true).on_hover_text(tr("Remove")).clicked() {
                                        confirm_remove = Some(r.name.clone());
                                        consumed = true;
                                    }
                                }
                                if !consumed && resp.double_clicked() {
                                    path = target.clone();
                                    go = true;
                                } else if !consumed && resp.clicked() {
                                    path = target.clone();
                                }
                            }
                        });
                        if content > list_h {
                            let track = list.height();
                            let th = (track * list_h / content).max(24.0);
                            let ty = list.top() + (track - th) * scroll / (content - list_h);
                            let bar = Rect::from_min_size(Pos2::new(list.right() + 8.0, ty), Vec2::new(4.0, th));
                            ui.painter().rect_filled(bar, cr(2.0), with_alpha(theme.text_faint, 0.5));
                        }
                    }
                    y += list_h + 24.0;
                    ui.painter().text(Pos2::new(x0, y), Align2::LEFT_TOP, tr("LOCATION"), bold(11.0), theme.text_faint);
                    y += 18.0;
                    let r = widgets::text_input(ui, Rect::from_min_size(Pos2::new(x0, y), Vec2::new(w, 38.0)), Id::new("rc-path"), &mut path, "remote:bucket/folder", &theme);
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        go = true;
                    }
                    let b = Rect::from_min_size(Pos2::new(card.right() - 150.0, foot_y), Vec2::new(122.0, 36.0));
                    go |= widgets::button_ex(ui, b, Id::new("rc-go"), tr("Scan"), Some(Icon::Bolt), BtnStyle::Primary, &theme, path.contains(':')).clicked();
                    let t = Rect::from_min_size(Pos2::new(x0 - 8.0, foot_y), Vec2::new(210.0, 36.0));
                    terminal = widgets::button(ui, t, Id::new("rc-term"), tr("Advanced setup (terminal)"), None, BtnStyle::Ghost, &theme).clicked();
                }
            }
        });
        if start_install {
            self.rclone_install = Some(rclone::install(ctx.clone()));
        }
        if cancel_install {
            if let Some(t) = &self.rclone_install {
                t.cancel();
            }
        }
        if other_ways {
            super::app::os_open("https://rclone.org/install/", false);
        }
        if refresh {
            refresh_rclone(&self.rclone, ctx);
        }
        if terminal {
            if let Err(e) = rclone::open_config_terminal() {
                return Some(Modal::Error { title: tr("Couldn't open a terminal").into(), message: e });
            }
        }
        if let Some(name) = remove {
            let _ = rclone::command().args(["config", "delete", &name]).stdin(std::process::Stdio::null()).status();
            if path.starts_with(&format!("{name}:")) {
                path.clear();
            }
            confirm_remove = None;
            refresh_rclone(&self.rclone, ctx);
        }
        if add {
            return Some(Modal::CloudAccount(Box::new(AccountForm::new(path))));
        }
        if let Some(r) = edit {
            return Some(Modal::CloudAccount(Box::new(AccountForm::edit(&r, path))));
        }
        if go && path.contains(':') {
            self.modal = None;
            self.start_session(Target::Rclone(path.trim().to_string()));
            return None;
        }
        if close || bd { None } else { Some(Modal::Rclone { path, confirm_remove }) }
    }

    pub(super) fn account_modal(&mut self, ctx: &egui::Context, mut form: Box<AccountForm>) -> Option<Modal> {
        let theme = self.theme.clone();
        let time = self.time;
        let existing = match &*self.rclone.lock().unwrap() {
            RcloneState::Ready(v) => v.clone(),
            _ => vec![],
        };
        // a background rclone command finished
        if let Some((t, what)) = form.task.clone() {
            if let Some(r) = t.finished() {
                form.task = None;
                if what == Pending::Save && form.original.is_none() && r.is_err() {
                    forget(form.name.trim().to_string(), &self.rclone, ctx);
                }
                match (what, r) {
                    (_, Err(e)) if e == "cancelled" => {}
                    (Pending::Save, Ok(())) => {
                        let oauth = form.provider.is_some_and(|i| PROVIDERS[i].oauth);
                        let name = form.name.trim().to_string();
                        refresh_rclone(&self.rclone, ctx);
                        if form.original.is_none() && !oauth {
                            // saved: now check we can actually connect
                            form.original = Some(name.clone());
                            form.loaded = form.values.clone();
                            form.task = Some((rclone::run(test_args(&name), ctx.clone()), Pending::Test));
                        } else {
                            return Some(Modal::Rclone { path: format!("{name}:"), confirm_remove: None });
                        }
                    }
                    (Pending::Test, Ok(())) | (Pending::Reconnect, Ok(())) => {
                        refresh_rclone(&self.rclone, ctx);
                        return Some(Modal::Rclone { path: format!("{}:", form.name.trim()), confirm_remove: None });
                    }
                    (Pending::Test, Err(e)) => form.error = Some(trf("Saved, but the connection test failed: {error}", &[("error", &e)])),
                    (_, Err(e)) => form.error = Some(e),
                }
            }
        }
        let busy = form.task.clone();
        let mut close = false;
        let mut back = false;
        let mut save = false;
        let mut reconnect = false;
        let mut cancel = false;
        let mut terminal = false;
        let mut picked: Option<usize> = None;
        let mut open_link: Option<String> = None;
        let fields: &[Field] = form.provider.map(|i| PROVIDERS[i].fields).unwrap_or(&[]);
        let oauth = form.provider.is_some_and(|i| PROVIDERS[i].oauth);
        let rows = fields.len().div_ceil(2) as f32;
        let height = if form.provider.is_none() && form.original.is_none() {
            100.0 + 4.0 * 66.0 + 80.0
        } else {
            100.0 + 70.0 + rows * 70.0 + if oauth || form.provider.is_none() { 90.0 } else { 0.0 } + 34.0 + 80.0
        };
        let bd = modal_frame(ctx, self.modal_opened, Vec2::new(600.0, height), &theme, |ui, card| {
            let x0 = card.left() + 28.0;
            let w = card.width() - 56.0;
            let foot_y = card.bottom() - 60.0;
            close = close_button(ui, card, &theme);
            let bk = Rect::from_min_size(Pos2::new(x0 - 8.0, foot_y), Vec2::new(100.0, 36.0));
            // --- provider grid
            if form.provider.is_none() && form.original.is_none() {
                title(ui, card, tr("Add a cloud account"), Some(tr("Pick where your files live.")), &theme);
                let cols = 3;
                let gap = 10.0;
                let tw = (w - gap * 2.0) / cols as f32;
                let th = 56.0;
                let y0 = card.top() + 100.0;
                for (k, p) in PROVIDERS.iter().enumerate().map(|(k, p)| (k, Some(p))).chain(std::iter::once((PROVIDERS.len(), None))) {
                    let r = Rect::from_min_size(Pos2::new(x0 + (k % cols) as f32 * (tw + gap), y0 + (k / cols) as f32 * (th + gap)), Vec2::new(tw, th));
                    let id = Id::new(("prov", k));
                    let resp = ui.interact(r, id, Sense::click());
                    let h = ui.ctx().animate_bool_with_time(id, resp.hovered(), 0.1);
                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    let (label, sub, icon) = match p {
                        Some(p) => (p.label, if p.oauth { tr("Sign in with your browser") } else { tr("Keys or password") }, p.icon),
                        None => (tr("Something else"), tr("70+ more, in the terminal"), Icon::LinkOut),
                    };
                    let pn = ui.painter();
                    pn.rect_filled(r, cr(8.0), lerp_color(theme.surface, theme.surface_hi, h));
                    pn.rect_stroke(r, cr(8.0), Stroke::new(1.0, lerp_color(theme.stroke, theme.accent, h)), egui::StrokeKind::Inside);
                    widgets::draw_icon(pn, icon, Rect::from_center_size(Pos2::new(r.left() + 20.0, r.center().y), Vec2::splat(16.0)), lerp_color(theme.text_dim, theme.accent, h));
                    let lg = pn.layout_no_wrap(label.to_string(), bold(12.5), theme.text);
                    let clip = pn.with_clip_rect(r.shrink(2.0));
                    clip.galley(Pos2::new(r.left() + 38.0, r.center().y - 9.0 - lg.size().y / 2.0), lg, theme.text);
                    clip.text(Pos2::new(r.left() + 38.0, r.center().y + 9.0), Align2::LEFT_CENTER, sub, font(11.0), theme.text_dim);
                    mark(format!("prov:{}", p.map(|p| p.kind).unwrap_or("other")), r);
                    if resp.clicked() {
                        match p {
                            Some(_) => picked = Some(k),
                            None => terminal = true,
                        }
                    }
                }
                back = widgets::button(ui, bk, Id::new("acc-back"), tr("Back"), Some(Icon::Back), BtnStyle::Ghost, &theme).clicked();
                return;
            }

            // --- form
            let heading = match (&form.original, form.provider) {
                (Some(n), _) => trf("Edit {name}", &[("name", n)]),
                (None, Some(i)) => PROVIDERS[i].label.to_string(),
                _ => String::new(),
            };
            let sub = match (&form.original, oauth) {
                (Some(_), _) => kind_label(&form.kind),
                (None, true) => tr("Sign in with your browser").to_string(),
                (None, false) => tr("Enter the details from your provider.").to_string(),
            };
            title(ui, card, &heading, Some(&sub), &theme);
            let mut y = card.top() + 100.0;
            let enabled = busy.is_none();
            ui.painter().text(Pos2::new(x0, y), Align2::LEFT_TOP, tr("NAME"), bold(11.0), theme.text_faint);
            y += 18.0;
            let nr = Rect::from_min_size(Pos2::new(x0, y), Vec2::new((w - 12.0) / 2.0, 38.0));
            ui.add_enabled_ui(enabled, |ui| {
                widgets::text_input(ui, nr, Id::new("acc-name"), &mut form.name, "gdrive", &theme);
            });
            let nm = form.name.trim().to_string();
            let taken = existing.iter().any(|r| r.name == nm) && form.original.as_deref() != Some(nm.as_str());
            let hint = if taken {
                Some((tr("That name is already used"), theme.danger))
            } else if !nm.is_empty() && !rclone::valid_name(&nm) {
                Some((tr("Letters, numbers, - _ . and spaces only"), theme.danger))
            } else {
                Some((tr("Shown in the list; you'll scan it as name:folder"), theme.text_faint))
            };
            if let Some((h, c)) = hint {
                ui.painter().text(Pos2::new(nr.right() + 14.0, nr.center().y), Align2::LEFT_CENTER, h, font(11.5), c);
            }
            y += 52.0;
            let cw = (w - 12.0) / 2.0;
            let editing = form.original.is_some();
            for (k, fl) in fields.iter().enumerate() {
                let fx = x0 + (k % 2) as f32 * (cw + 12.0);
                let fy = y + (k / 2) as f32 * 70.0;
                let label = if fl.optional { format!("{} · {}", tr(fl.label).to_uppercase(), tr("optional")) } else { tr(fl.label).to_uppercase() };
                ui.painter().text(Pos2::new(fx, fy), Align2::LEFT_TOP, label, bold(11.0), theme.text_faint);
                let r = Rect::from_min_size(Pos2::new(fx, fy + 18.0), Vec2::new(cw, 38.0));
                let id = Id::new(("acc-f", fl.key));
                let hint = if fl.secret && editing { tr("Unchanged") } else { tr(fl.hint) };
                ui.add_enabled_ui(enabled, |ui| {
                    if fl.secret {
                        secret_input(ui, r, id, &mut form.values[k], hint, &theme);
                    } else {
                        widgets::text_input(ui, r, id, &mut form.values[k], hint, &theme);
                    }
                });
            }
            y += rows * 70.0;
            if oauth || form.provider.is_none() {
                let msg = if form.provider.is_none() {
                    tr("SquirrelDisk can rename this account. To change its other settings, use the advanced setup in a terminal.")
                } else if editing {
                    tr("Sign in again if the connection stopped working or you want to switch account.")
                } else {
                    tr("Your browser will open so you can sign in. SquirrelDisk never sees your password: rclone keeps the access token on this computer.")
                };
                y += wrapped(ui, Pos2::new(x0, y + 4.0), w, msg, font(12.5), theme.text_dim) + 16.0;
                if editing && oauth {
                    let rb = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(170.0, 34.0));
                    reconnect = widgets::button_ex(ui, rb, Id::new("acc-reconnect"), tr("Sign in again"), Some(Icon::Refresh), BtnStyle::Subtle, &theme, enabled).clicked();
                } else if form.provider.is_none() {
                    let rb = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(220.0, 34.0));
                    terminal = widgets::button(ui, rb, Id::new("acc-term"), tr("Advanced setup (terminal)"), Some(Icon::LinkOut), BtnStyle::Subtle, &theme).clicked();
                }
                y += 44.0;
            }
            // status line
            if let Some((t, what)) = &busy {
                let msg = match what {
                    Pending::Save if oauth && !editing => tr("Finish signing in in your browser…"),
                    Pending::Reconnect => tr("Finish signing in in your browser…"),
                    Pending::Test => tr("Checking the connection…"),
                    Pending::Save => tr("Saving…"),
                };
                spinner(ui, Pos2::new(x0 + 8.0, y + 10.0), 7.0, theme.accent, time);
                ui.painter().text(Pos2::new(x0 + 26.0, y + 10.0), Align2::LEFT_CENTER, msg, font(12.5), theme.text);
                // the browser didn't show up? offer rclone's sign-in link
                if let Some(link) = t.link.lock().unwrap().clone() {
                    let lb = Rect::from_min_size(Pos2::new(x0 - 8.0 + 26.0, y + 22.0), Vec2::new(200.0, 30.0));
                    if widgets::button(ui, lb, Id::new("acc-link"), tr("Open the sign-in page"), Some(Icon::LinkOut), BtnStyle::Ghost, &theme).clicked() {
                        open_link = Some(link);
                    }
                }
            } else if let Some(e) = &form.error {
                wrapped(ui, Pos2::new(x0, y), w, e, font(12.0), theme.danger);
            }
            if busy.is_some() {
                let c = Rect::from_min_size(Pos2::new(card.right() - 128.0, foot_y), Vec2::new(100.0, 36.0));
                cancel = widgets::button(ui, c, Id::new("acc-cancel"), tr("Cancel"), None, BtnStyle::Ghost, &theme).clicked();
            } else {
                let label = if oauth && !editing { tr("Sign in") } else { tr("Save") };
                let b = Rect::from_min_size(Pos2::new(card.right() - 160.0, foot_y), Vec2::new(132.0, 36.0));
                mark("acc-save", b);
                save = widgets::button_ex(ui, b, Id::new("acc-save"), label, Some(if oauth && !editing { Icon::LinkOut } else { Icon::Check }), BtnStyle::Primary, &theme, form.ready() && !taken)
                    .clicked();
            }
            back = widgets::button(ui, bk, Id::new("acc-back"), tr("Back"), Some(Icon::Back), BtnStyle::Ghost, &theme).clicked();
        });
        if let Some(i) = picked {
            form.pick(i, &existing);
        }
        if let Some(l) = open_link {
            super::app::os_open(&l, false);
        }
        if terminal {
            if let Err(e) = rclone::open_config_terminal() {
                form.error = Some(e);
            }
        }
        if cancel {
            form.cancel(&self.rclone, ctx);
        }
        if save && form.task.is_none() {
            form.error = None;
            let steps = form.save_steps();
            if steps.is_empty() {
                return Some(Modal::Rclone { path: format!("{}:", form.name.trim()), confirm_remove: None });
            }
            form.task = Some((rclone::run_all(steps, ctx.clone()), Pending::Save));
        }
        if reconnect && form.task.is_none() {
            form.error = None;
            let name = form.original.clone().unwrap_or_default();
            form.task = Some((rclone::run(vec!["config".into(), "reconnect".into(), format!("{name}:")], ctx.clone()), Pending::Reconnect));
        }
        if back {
            form.cancel(&self.rclone, ctx);
            if form.provider.is_some() && form.original.is_none() {
                form.provider = None;
                return Some(Modal::CloudAccount(form));
            }
            return Some(Modal::Rclone { path: form.back_path.clone(), confirm_remove: None });
        }
        if close || bd {
            form.cancel(&self.rclone, ctx);
            return None;
        }
        Some(Modal::CloudAccount(form))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait(t: Arc<Task>) -> Result<(), String> {
        while t.finished().is_none() {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        t.finished().unwrap()
    }

    /// Needs rclone; uses a throwaway config:
    /// `RCLONE_CONFIG=/tmp/x.conf cargo test -- --ignored account_roundtrip`
    #[test]
    #[ignore]
    fn account_roundtrip() {
        assert!(std::env::var("RCLONE_CONFIG").is_ok_and(|c| c.contains("tmp") || c.contains("scratch")), "use a throwaway RCLONE_CONFIG");
        let ctx = egui::Context::default();
        let mut f = AccountForm::with_provider("sftp", &[]);
        f.name = "sqd test".into();
        f.values = vec!["example.invalid".into(), "me".into(), "hunter2".into(), "".into(), "".into()];
        wait(rclone::run_all(f.save_steps(), ctx.clone())).unwrap();
        let r = rclone::remotes().into_iter().find(|r| r.name == "sqd test").expect("created");
        assert_eq!(r.param("host"), Some("example.invalid"));
        assert_ne!(r.param("pass"), Some("hunter2"), "password must be obscured");

        // rename + change port, password untouched
        let mut e = AccountForm::edit(&r, String::new());
        e.name = "sqd-renamed".into();
        e.values[3] = "2222".into();
        wait(rclone::run_all(e.save_steps(), ctx.clone())).unwrap();
        let all = rclone::remotes();
        assert!(all.iter().all(|r| r.name != "sqd test"), "old name gone");
        let n = all.iter().find(|r| r.name == "sqd-renamed").expect("renamed");
        assert_eq!(n.param("port"), Some("2222"));
        assert_eq!(n.param("pass"), r.param("pass"), "password kept");
        let _ = rclone::command().args(["config", "delete", "sqd-renamed"]).status();
    }
}
