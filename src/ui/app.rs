use super::fx::{Drag, Particles, Toasts};
use super::shader::SunburstGl;
use super::sunburst::{Animator, View};
use super::theme::{self, Theme};
use super::widgets;
use crate::delete::{self, DeleteProgress};
use crate::disks::{self, Volume};
use crate::safety::Verdict;
use crate::scan::{self, Progress, SharedTree};
use crate::settings::Settings;
use crate::sound::{Sfx, Sounds};
use crate::tree::{fmt_size, Kind, Source};
use crate::update::Updater;
use crate::watch::FsWatch;
use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, Pos2, Rect};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Volume(Volume),
    Folder(PathBuf),
    Ssh { host: String, path: String },
    Rclone(String),
}

impl Target {
    pub fn title(&self) -> String {
        match self {
            Target::Volume(v) => v.name.clone(),
            Target::Folder(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string_lossy().into_owned()),
            Target::Ssh { host, path } => format!("{host}:{path}"),
            Target::Rclone(r) => r.clone(),
        }
    }
    pub fn same_place(&self, other: &Target) -> bool {
        match (self, other) {
            (Target::Volume(a), Target::Volume(b)) => a.mount == b.mount,
            _ => self == other,
        }
    }
}

pub struct Session {
    pub target: Target,
    pub title: String,
    pub tree: SharedTree,
    pub progress: Arc<Progress>,
    pub view: View,
    pub back: Vec<View>,
    pub fwd: Vec<View>,
    pub anim: Animator,
    pub layout_sig: (u64, View, usize, i32, String),
    pub last_layout: Instant,
    pub collector: Vec<u32>,
    /// Node whose content is shown in the side panel.
    pub panel_view: View,
    pub hovered_key: Option<u64>,
    pub list_hover: Option<u32>,
    pub selected: Option<u32>,
    pub watch: Option<FsWatch>,
    pub pending: HashSet<PathBuf>,
    pub last_event: Instant,
    pub refreshing: Arc<AtomicBool>,
    pub delete: Option<Arc<DeleteProgress>>,
    pub intro: bool,
    pub was_done: bool,
    pub started: Instant,
    pub verts: Vec<f32>,
    pub idx: Vec<u32>,
    pub watch_focus: Option<View>,
    pub expanding: Arc<AtomicBool>,
    /// Last chart geometry (used by the demo director to aim at slices).
    pub geo: Option<super::sunburst::Geometry>,
    /// Free bytes on the scanned volume (disk scans only), refreshed periodically.
    pub free_space: u64,
    pub free_at: Instant,
}

impl Session {
    pub fn new(target: Target) -> Session {
        let handle = match &target {
            _ if scan::demo::enabled() => match &target {
                Target::Volume(v) => scan::demo::start(v.name.clone(), v.mount.clone()),
                other => scan::demo::start(other.title(), "/Volumes/Demo".into()),
            },
            Target::Volume(v) => scan::local::start(PathBuf::from(&v.mount), v.name.clone()),
            Target::Folder(p) => scan::local::start(p.clone(), target.title()),
            Target::Ssh { host, path } => scan::remote::start_ssh(host.clone(), path.clone()),
            Target::Rclone(r) => scan::remote::start_rclone(r.clone()),
        };
        let root = View::node(0);
        Session {
            title: target.title(),
            target,
            tree: handle.tree,
            progress: handle.progress,
            view: root,
            back: Vec::new(),
            fwd: Vec::new(),
            anim: Animator::default(),
            layout_sig: (0, root, 0, 0, String::new()),
            last_layout: Instant::now() - Duration::from_secs(1),
            collector: Vec::new(),
            panel_view: root,
            hovered_key: None,
            list_hover: None,
            selected: None,
            watch: None,
            pending: HashSet::new(),
            last_event: Instant::now(),
            refreshing: Arc::new(AtomicBool::new(false)),
            delete: None,
            intro: true,
            was_done: false,
            started: Instant::now(),
            verts: Vec::new(),
            idx: Vec::new(),
            watch_focus: None,
            expanding: Arc::new(AtomicBool::new(false)),
            geo: None,
            free_space: 0,
            free_at: Instant::now() - Duration::from_secs(60),
        }
    }

    pub fn is_scanning(&self) -> bool {
        !self.progress.is_done()
    }

    pub fn source(&self) -> Source {
        self.tree.read().unwrap().source.clone()
    }

    pub fn navigate(&mut self, v: View) {
        if v != self.view {
            self.back.push(self.view);
            self.fwd.clear();
            self.view = v;
            self.panel_view = v;
        }
    }

    pub fn go_back(&mut self) -> bool {
        if let Some(v) = self.back.pop() {
            self.fwd.push(self.view);
            self.view = v;
            self.panel_view = v;
            true
        } else {
            false
        }
    }

    pub fn go_forward(&mut self) -> bool {
        if let Some(v) = self.fwd.pop() {
            self.back.push(self.view);
            self.view = v;
            self.panel_view = v;
            true
        } else {
            false
        }
    }

    pub fn go_up(&mut self) -> bool {
        let t = self.tree.read().unwrap();
        let parent = if self.view.skip > 0 { self.view.node } else { t.get(self.view.node).parent };
        drop(t);
        if parent == crate::tree::NONE {
            return false;
        }
        self.navigate(View::node(parent));
        true
    }

    /// Adds a node to the collector, keeping the set minimal.
    /// Returns false if already covered.
    pub fn collect(&mut self, node: u32) -> bool {
        let t = self.tree.read().unwrap();
        if !t.is_alive(node) || node == t.root {
            return false;
        }
        if matches!(t.get(node).kind, Kind::Hidden | Kind::Mount) {
            return false;
        }
        if self.collector.iter().any(|c| *c == node || t.is_ancestor(*c, node)) {
            return false;
        }
        let keep: Vec<u32> = self.collector.iter().copied().filter(|c| !t.is_ancestor(node, *c)).collect();
        drop(t);
        self.collector = keep;
        self.collector.push(node);
        true
    }

    pub fn collected_size(&self) -> u64 {
        let t = self.tree.read().unwrap();
        self.collector.iter().filter(|c| t.is_alive(**c)).map(|c| t.get(*c).size).sum()
    }

    pub fn is_collected(&self, t: &crate::tree::Tree, node: u32) -> bool {
        self.collector.iter().any(|c| *c == node || t.is_ancestor(*c, node))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Home,
    Session(usize),
}

pub struct DeleteItem {
    pub node: u32,
    pub path: String,
    pub name: String,
    pub size: u64,
    pub verdict: Verdict,
}

pub enum Modal {
    Settings,
    Ssh { host: String, path: String },
    Rclone { path: String },
    ConfirmDelete { session: usize, items: Vec<DeleteItem>, mode: usize, backup_folder: String, backup_remote: String, ack: bool, secure: bool },
    Deleting { session: usize, finished_at: Option<Instant> },
    Error { title: String, message: String },
    /// ssh needs a key passphrase or a password.
    SshAuth { host: String, path: String, secret: String, retry: bool },
}

/// Drives the animated Dock tile on macOS.
#[derive(Default)]
pub struct DockState {
    shown: bool,
    scanning: bool,
    last: Option<Instant>,
    done_at: Option<Instant>,
}

pub struct CtxMenu {
    pub session: usize,
    pub node: u32,
    pub pos: Pos2,
    pub opened: Instant,
}

pub struct App {
    pub settings: Settings,
    pub theme: Theme,
    pub themes: Vec<Theme>,
    pub sounds: Option<Sounds>,
    pub gl: Option<Arc<Mutex<SunburstGl>>>,
    pub screen: Screen,
    pub sessions: Vec<Session>,
    pub volumes: Vec<Volume>,
    pub volumes_at: Instant,
    pub updater: Updater,
    pub modal: Option<Modal>,
    pub modal_opened: Instant,
    pub toasts: Toasts,
    pub drag: Option<Drag>,
    pub particles: Particles,
    pub time: f64,
    pub dt: f32,
    pub last_frame: Instant,
    pub rclone: Arc<Mutex<Option<Vec<String>>>>,
    pub ssh_hosts: Vec<String>,
    pub ctx_menu: Option<CtxMenu>,
    pub bin_rect: Rect,
    pub bin_open: bool,
    pub bin_bounce: f32,
    pub applied_theme: String,
    pub gl_error: Option<String>,
    pub debug: super::debug::Debug,
    /// (vertical center, right edge) of the macOS traffic lights, in points.
    pub chrome: (f32, f32),
    /// Hosts for which the user already typed a secret (to flag wrong ones).
    pub ssh_tried: HashSet<String>,
    /// Language picker popover open in Settings.
    pub lang_picker: bool,
    /// Settings modal scroll offset and measured content height.
    pub settings_scroll: f32,
    pub settings_content_h: f32,
    /// Home page scroll offset and measured content height.
    pub home_scroll: f32,
    pub home_content_h: f32,
    pub sponsors: crate::sponsor::Sponsors,
    /// App icon as a texture, for the home header.
    pub logo: Option<egui::TextureHandle>,
    pub dock: DockState,
    pub director: Option<super::director::Director>,
    /// Named rects of clickable things, for the demo director.
    pub marks: std::collections::HashMap<String, Rect>,
}

/// Installs the bundled fonts plus, for non-Latin languages, a system font
/// that covers the script (loaded at runtime to keep the app small).
pub fn install_fonts(ctx: &egui::Context, lang: &str) {
    let mut fonts = FontDefinitions::default();
    let script_font = crate::i18n::system_font(lang).map(|(bytes, index)| {
        let mut fd = FontData::from_owned(bytes);
        fd.index = index;
        fd
    });
    let has_script = script_font.is_some();
    if let Some(fd) = script_font {
        fonts.font_data.insert("system-script".into(), Arc::new(fd));
    }
    fonts.font_data.insert("inter".into(), Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Inter-Regular.ttf"))));
    fonts.font_data.insert("inter-bold".into(), Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"))));
    fonts.font_data.insert("fredoka".into(), Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Fredoka-SemiBold.ttf"))));
    let mut fallback: Vec<String> = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if has_script {
        fallback.insert(0, "system-script".into());
    }
    let mut prop = vec!["inter".to_string()];
    prop.extend(fallback.iter().cloned());
    fonts.families.insert(FontFamily::Proportional, prop);
    let mut b = vec!["inter-bold".to_string()];
    b.extend(fallback.iter().cloned());
    fonts.families.insert(FontFamily::Name("bold".into()), b);
    let mut d = vec!["inter-bold".to_string()];
    d.extend(fallback.iter().cloned());
    fonts.families.insert(FontFamily::Name("display".into()), d);
    let mut br = vec!["fredoka".to_string(), "inter-bold".to_string()];
    br.extend(fallback.iter().cloned());
    fonts.families.insert(FontFamily::Name("brand".into()), br);
    ctx.set_fonts(fonts);
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = Settings::load();
        let lang = crate::i18n::resolve(&std::env::var("SQUIRRELDISK_LANG").unwrap_or_else(|_| settings.language.clone()));
        crate::i18n::set_language(lang);
        install_fonts(&cc.egui_ctx, lang);
        let theme = theme::by_name(&settings.theme);
        let (gl, gl_error) = match cc.gl.as_ref().map(|gl| SunburstGl::new(gl)) {
            Some(Ok(r)) => (Some(Arc::new(Mutex::new(r))), None),
            Some(Err(e)) => (None, Some(e)),
            None => (None, Some("no OpenGL context".into())),
        };
        if let Some(e) = &gl_error {
            eprintln!("SquirrelDisk: falling back to CPU rendering: {e}");
        }
        let rclone = Arc::new(Mutex::new(None));
        {
            let r = rclone.clone();
            std::thread::spawn(move || {
                let v = if scan::remote::rclone_available() { Some(scan::remote::rclone_remotes()) } else { None };
                *r.lock().unwrap() = Some(v.unwrap_or_else(|| vec!["\u{0}".into()]));
            });
        }
        let updater = Updater::new();
        if settings.auto_update && !scan::demo::enabled() {
            updater.check(cc.egui_ctx.clone());
        }
        App {
            themes: theme::all(),
            theme,
            sounds: Some(Sounds::new()),
            gl,
            screen: Screen::Home,
            sessions: Vec::new(),
            volumes: disks::list(),
            volumes_at: Instant::now(),
            updater,
            modal: None,
            modal_opened: Instant::now(),
            toasts: Toasts::default(),
            drag: None,
            particles: Particles::default(),
            time: 0.0,
            dt: 0.016,
            last_frame: Instant::now(),
            rclone,
            ssh_hosts: scan::remote::ssh_config_hosts(),
            ctx_menu: None,
            bin_rect: Rect::NOTHING,
            bin_open: false,
            bin_bounce: 0.0,
            applied_theme: String::new(),
            gl_error,
            debug: super::debug::Debug::from_env(),
            ssh_tried: HashSet::new(),
            lang_picker: false,
            settings_scroll: 0.0,
            settings_content_h: 640.0,
            home_scroll: 0.0,
            home_content_h: 0.0,
            sponsors: crate::sponsor::Sponsors::start(settings.sponsor_measurement),
            logo: None,
            dock: DockState::default(),
            director: super::director::Director::from_env(),
            marks: Default::default(),
            chrome: if cfg!(target_os = "macos") { (15.0, 68.0) } else { (22.0, 0.0) },
            settings,
        }
    }

    /// Feeds the local sponsor matcher with the startup disk fill level.
    pub fn update_sponsor_disk(&self) {
        let boot = self.volumes.iter().find(|v| v.is_boot).map(|v| v.used_frac());
        let external = self.volumes.iter().any(|v| v.removable);
        self.sponsors.set_disk(boot, external);
    }

    /// Draws the sponsor banner and handles views/clicks.
    pub fn sponsor_banner(&mut self, ui: &mut egui::Ui, rect: Rect, placement: &'static str) {
        let personalized = self.settings.personalized_sponsors;
        let measure = self.settings.sponsor_measurement;
        let choice = self.sponsors.choose(personalized);
        self.sponsors.record_view(&choice.ad, placement, measure);
        let theme = self.theme.clone();
        if let super::ads::BannerAction::Open = super::ads::banner(ui, rect, egui::Id::new(("sponsor", placement)), &choice, personalized, &theme) {
            os_open(&self.sponsors.click_url(&choice.ad, placement, measure), false);
        }
    }

    /// Mirrors scan progress in the Dock icon: the ring fills up and the acorn
    /// wobbles while scanning, then it hops once and returns to the full icon.
    #[allow(unused_variables)]
    fn update_dock(&mut self, ctx: &egui::Context) {
        #[cfg(target_os = "macos")]
        {
            use crate::icon;
            let now = Instant::now();
            let due = |every: u64, d: &DockState| d.last.is_none_or(|l| now.duration_since(l).as_millis() as u64 >= every);
            let focus = match self.screen {
                Screen::Session(i) => self.sessions.get(i).filter(|s| s.is_scanning()),
                Screen::Home => None,
            };
            let scanning = focus.or_else(|| self.sessions.iter().find(|s| s.is_scanning()));
            let frame = if let Some(s) = scanning {
                self.dock.scanning = true;
                self.dock.done_at = None;
                due(80, &self.dock).then(|| icon::scanning_frame(s.progress.fraction(), self.time as f32))
            } else {
                if self.dock.scanning {
                    self.dock.scanning = false;
                    self.dock.done_at = Some(now);
                }
                match self.dock.done_at {
                    Some(d) if d.elapsed().as_secs_f32() < 1.2 => {
                        ctx.request_repaint();
                        due(40, &self.dock).then(|| icon::done_frame(d.elapsed().as_secs_f32()))
                    }
                    Some(_) => {
                        self.dock.done_at = None;
                        Some(icon::Frame::IDLE)
                    }
                    None if !self.dock.shown => Some(icon::Frame::IDLE),
                    None => None,
                }
            };
            if let Some(f) = frame {
                super::macos::set_dock_icon(&icon::png(256, f, true));
                self.dock.shown = true;
                self.dock.last = Some(now);
            }
        }
    }

    /// Registers a clickable rect (only needed by the demo director).
    pub fn mark(&mut self, name: impl Into<String>, rect: Rect) {
        if self.director.is_some() {
            self.marks.insert(name.into(), rect);
        }
    }

    /// Switches the UI language ("auto" follows the system).
    pub fn set_language(&mut self, ctx: &egui::Context, setting: &str) {
        self.settings.language = setting.to_string();
        let code = crate::i18n::resolve(setting);
        crate::i18n::set_language(code);
        install_fonts(ctx, code);
        self.settings.save();
    }

    pub fn sfx(&mut self, s: Sfx) {
        if self.settings.sound {
            let vol = self.settings.volume;
            if let Some(snd) = self.sounds.as_mut() {
                snd.play(s, vol);
            }
        }
    }

    pub fn open_modal(&mut self, m: Modal) {
        self.modal = Some(m);
        self.modal_opened = Instant::now();
    }

    pub fn start_session(&mut self, target: Target) {
        if let Some(i) = self.sessions.iter().position(|s| s.target.same_place(&target)) {
            if self.sessions[i].is_scanning() {
                self.screen = Screen::Session(i);
                return;
            }
            self.sessions[i] = Session::new(target);
            self.screen = Screen::Session(i);
        } else {
            self.sessions.push(Session::new(target));
            self.screen = Screen::Session(self.sessions.len() - 1);
        }
        self.sfx(Sfx::Blip);
    }

    pub fn cancel_session(&mut self, i: usize) {
        if let Some(s) = self.sessions.get(i) {
            s.progress.cancel.store(true, Ordering::Relaxed);
        }
        self.sessions.remove(i);
        self.screen = Screen::Home;
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        if self.applied_theme == self.theme.name {
            return;
        }
        self.applied_theme = self.theme.name.to_string();
        let t = &self.theme;
        let mut v = if t.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        v.selection.bg_fill = theme::with_alpha(t.accent, 0.45);
        v.selection.stroke = egui::Stroke::new(1.0, t.text);
        v.text_cursor.stroke = egui::Stroke::new(2.0, t.accent);
        v.extreme_bg_color = t.bg_bottom;
        v.panel_fill = t.bg_bottom;
        v.window_fill = t.surface;
        v.override_text_color = Some(t.text);
        ctx.set_visuals(v);
        ctx.global_style_mut(|s| {
            s.interaction.selectable_labels = false;
            s.spacing.item_spacing = egui::vec2(0.0, 0.0);
        });
    }

    /// Per-frame housekeeping for all sessions: watcher events, deletions, scan completion.
    fn poll_sessions(&mut self) {
        let mut sounds: Vec<Sfx> = Vec::new();
        let mut toasts: Vec<(String, Color32)> = Vec::new();
        let mut auth_needed: Option<(String, String)> = None;
        let mut interest_jobs: Vec<SharedTree> = Vec::new();
        let watch_enabled = self.settings.watch_fs;
        for s in self.sessions.iter_mut() {
            // scan finished?
            if !s.was_done && s.progress.is_done() {
                s.was_done = true;
                if let Some(e) = s.progress.error.lock().unwrap().clone() {
                    if let (Target::Ssh { host, path }, true) = (&s.target, scan::remote::is_auth_error(&e)) {
                        auth_needed = Some((host.clone(), path.clone()));
                    } else if e != "cancelled" {
                        toasts.push((format!("{}: {e}", s.title), self.theme.danger));
                        sounds.push(Sfx::Error);
                    }
                } else {
                    let (size, files) = {
                        let t = s.tree.read().unwrap();
                        (t.get(t.root).size, t.get(t.root).files)
                    };
                    toasts.push((
                        crate::i18n::trf("{name} scanned · {size} in {files} files · {seconds}s", &[("name", &s.title), ("size", &fmt_size(size)), ("files", &crate::tree::fmt_count(files as u64)), ("seconds", &format!("{:.1}", s.started.elapsed().as_secs_f32()))]),
                        self.theme.ok,
                    ));
                    sounds.push(Sfx::Success);
                    if s.source().is_local() {
                        interest_jobs.push(s.tree.clone());
                    }
                    if watch_enabled && s.source().is_local() {
                        let root = s.tree.read().unwrap().root_path.clone();
                        s.watch = FsWatch::new(std::path::Path::new(&root));
                    }
                }
            }
            // filesystem events
            if let Some(w) = &s.watch {
                let root = s.tree.read().unwrap().root_path.clone();
                let mut got = false;
                while let Ok(p) = w.rx.try_recv() {
                    if s.pending.len() < 4096 {
                        s.pending.insert(crate::watch::normalize_event_path(p, &root));
                    }
                    got = true;
                }
                if got {
                    s.last_event = Instant::now();
                }
            }
            if !s.pending.is_empty() && s.last_event.elapsed() > Duration::from_millis(600) && !s.refreshing.load(Ordering::Relaxed) && s.delete.is_none() {
                let paths: Vec<PathBuf> = s.pending.drain().collect();
                let tree = s.tree.clone();
                let flag = s.refreshing.clone();
                flag.store(true, Ordering::Relaxed);
                std::thread::spawn(move || {
                    apply_fs_changes(&tree, paths);
                    flag.store(false, Ordering::Relaxed);
                });
            }
            // deletion progress
            if let Some(d) = &s.delete {
                let done: Vec<u32> = std::mem::take(&mut *d.completed.lock().unwrap());
                if !done.is_empty() {
                    let mut t = s.tree.write().unwrap();
                    for n in &done {
                        t.remove(*n);
                    }
                    drop(t);
                    s.collector.retain(|c| !done.contains(c));
                    sounds.push(Sfx::Crunch);
                }
                if d.finished.load(Ordering::Relaxed) && d.completed.lock().unwrap().is_empty() {
                    // keep the job around for the modal until it is closed
                }
            }
        }
        for (t, c) in toasts {
            self.toasts.push(t, c);
        }
        if self.settings.personalized_sponsors {
            for tree in interest_jobs {
                // On-device only: folder names + sizes already in memory.
                self.sponsors.learn_from_scan(tree);
            }
        }
        if let Some((host, path)) = auth_needed {
            let retry = self.ssh_tried.contains(&host);
            self.ssh_tried.insert(host.clone());
            self.open_modal(Modal::SshAuth { host, path, secret: String::new(), retry });
        }
        for snd in sounds {
            self.sfx(snd);
        }
    }

    pub fn start_delete(&mut self, session: usize, items: Vec<DeleteItem>, mode: delete::Mode) {
        let Some(s) = self.sessions.get_mut(session) else { return };
        let (source, root) = {
            let t = s.tree.read().unwrap();
            (t.source.clone(), t.root_path.clone())
        };
        let list: Vec<delete::Item> = items
            .into_iter()
            .filter(|i| !i.verdict.is_forbidden())
            .map(|i| delete::Item { node: i.node, path: i.path, size: i.size })
            .collect();
        s.delete = Some(delete::start(list, mode, source, root));
        self.open_modal(Modal::Deleting { session, finished_at: None });
    }
}

/// Maps a batch of changed paths to tree updates.
fn apply_fs_changes(tree: &SharedTree, paths: Vec<PathBuf>) {
    let mut dirs: HashSet<u32> = HashSet::new();
    let mut removed: Vec<u32> = Vec::new();
    {
        let t = tree.read().unwrap();
        for p in paths {
            let ps = p.to_string_lossy();
            let (id, exact) = t.find_path(&ps);
            if id == crate::tree::NONE {
                continue;
            }
            if exact && !p.exists() && id != t.root {
                removed.push(id);
                continue;
            }
            let dir = if exact && t.get(id).kind == Kind::Dir && !(p.exists() && !p.is_dir()) { id } else if exact { t.get(id).parent } else { id };
            if dir != crate::tree::NONE && t.get(dir).kind == Kind::Dir {
                dirs.insert(dir);
            }
        }
    }
    if !removed.is_empty() {
        let mut t = tree.write().unwrap();
        for r in removed {
            t.remove(r);
        }
    }
    for d in dirs.into_iter().take(64) {
        scan::local::refresh_dir(tree, d);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        #[cfg(target_os = "macos")]
        if let Some(c) = super::macos::traffic_lights(frame) {
            self.chrome = c;
        }
        let _ = &frame;
        let now = Instant::now();
        self.dt = now.duration_since(self.last_frame).as_secs_f32().clamp(0.001, 0.1);
        self.last_frame = now;
        self.time += self.dt as f64;
        self.apply_theme(&ctx);
        if self.time < 0.1 {
            self.update_sponsor_disk();
        }
        self.direct(&ctx);
        self.marks.clear();
        self.debug_hooks(&ctx);
        self.poll_sessions();
        self.update_dock(&ctx);
        if self.volumes_at.elapsed() > Duration::from_secs(5) && self.screen == Screen::Home {
            self.volumes = disks::list();
            self.volumes_at = Instant::now();
            self.update_sponsor_disk();
        }

        // dropped folders → scan
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if let Some(p) = dropped.into_iter().find(|p| p.is_dir()) {
            self.start_session(Target::Folder(p));
        }

        let screen = ctx.content_rect();
        widgets::vgradient(ui.painter(), screen, self.theme.bg_top, self.theme.bg_bottom);

        match self.screen {
            Screen::Home => self.home_ui(ui, screen),
            Screen::Session(i) => {
                if i < self.sessions.len() {
                    self.session_ui(ui, screen, i)
                } else {
                    self.screen = Screen::Home;
                }
            }
        }

        self.context_menu_ui(&ctx);
        self.modal_ui(&ctx);
        self.drag_overlay(&ctx);
        let theme = self.theme.clone();
        self.particles.step_and_draw(&ctx, self.dt, &theme);
        self.toasts.draw(&ctx, &theme, self.dt);

        // keyboard shortcuts
        let (esc, back) = ctx.input(|i| (i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::Backspace)));
        if esc {
            if self.ctx_menu.is_some() {
                self.ctx_menu = None;
            } else if self.modal.as_ref().is_some_and(|m| !matches!(m, Modal::Deleting { .. })) {
                self.modal = None;
            } else if self.drag.is_some() {
                self.drag = None;
            } else if let Screen::Session(i) = self.screen {
                if !self.sessions[i].go_up() {
                    self.screen = Screen::Home;
                } else {
                    self.sfx(Sfx::BlipDown);
                }
            }
        }
        if back && self.modal.is_none() && !ctx.egui_wants_keyboard_input() {
            if let Screen::Session(i) = self.screen {
                if self.sessions[i].go_up() {
                    self.sfx(Sfx::BlipDown);
                }
            }
        }

        // repaint policy: animate when something moves, poll slowly while scanning
        let scanning = self.sessions.iter().any(|s| s.is_scanning() || s.delete.is_some());
        let animating = self.drag.is_some()
            || !self.particles.is_empty()
            || !self.toasts.is_empty()
            || match self.screen {
                Screen::Session(i) => self.sessions.get(i).is_some_and(|s| !s.anim.settled),
                Screen::Home => false,
            };
        if animating || (self.settings.shader_fx && matches!(self.screen, Screen::Session(_))) {
            ctx.request_repaint();
        } else if scanning {
            ctx.request_repaint_after(Duration::from_millis(80));
        } else {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        if let Some(d) = self.director.as_mut() {
            d.inject(raw);
        }
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.settings.save();
        if let (Some(gl), Some(r)) = (gl, &self.gl) {
            if let Ok(r) = r.lock() {
                r.destroy(gl);
            }
        }
        for s in &self.sessions {
            s.progress.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let c = self.theme.bg_bottom;
        [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0, 1.0]
    }
}

/// Opens a path with the OS default handler, or reveals it in the file manager.
pub fn os_open(path: &str, reveal: bool) {
    let mut cmd;
    #[cfg(target_os = "macos")]
    {
        cmd = std::process::Command::new("open");
        if reveal {
            cmd.arg("-R");
        }
        cmd.arg(path);
    }
    #[cfg(windows)]
    {
        cmd = std::process::Command::new("explorer");
        if reveal {
            cmd.arg(format!("/select,{path}"));
        } else {
            cmd.arg(path);
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        cmd = std::process::Command::new("xdg-open");
        let p = std::path::Path::new(path);
        if reveal {
            cmd.arg(p.parent().unwrap_or(p));
        } else {
            cmd.arg(p);
        }
    }
    let _ = cmd.spawn();
}
