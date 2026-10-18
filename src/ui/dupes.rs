//! Duplicate finder screen: the places being compared, the groups of
//! identical files (each one opens on its copies), the "always keep" rules
//! and a preview of the selected file.

use super::app::{App, Modal, Screen, Target};
use super::theme::{from_hsl, lerp_color, lighten, with_alpha, Theme};
use super::widgets::{self, bold, cr, display, font, BtnStyle, Icon};
use crate::dupes::engine::{Finder, FoundGroup, Options, Spec, Status};
#[cfg(test)]
use crate::dupes::engine::FoundFile;
use crate::dupes::{apply_rules, mock, remove, ymd, Copy, Group, Media, Meta, Place, PlaceKind, Prefer, Rule, Scan};
use crate::i18n::{tr, trf};
use crate::sound::Sfx;
use crate::tree::{fmt_count, fmt_count_compact, fmt_size};
use eframe::egui::{self, epaint::RectShape, Align2, Color32, Id, Painter, Pos2, Rect, Sense, Stroke, Ui, Vec2};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Photos,
    Videos,
    Other,
}

impl Filter {
    fn accepts(self, m: Media) -> bool {
        match self {
            Filter::All => true,
            Filter::Photos => matches!(m, Media::Photo | Media::Raw),
            Filter::Videos => m == Media::Video,
            Filter::Other => matches!(m, Media::Audio | Media::Other),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Size,
    Copies,
    Name,
}

/// One line of the list: a group, or one of its copies.
#[derive(Clone, Copy, PartialEq)]
struct Line {
    group: usize,
    copy: Option<usize>,
    y: f32,
    h: f32,
}

const GROUP_H: f32 = 52.0;
const COPY_H: f32 = 38.0;
const GROUP_GAP: f32 = 6.0;

/// The comparison behind the screen.
enum Run {
    /// Not started yet.
    Idle,
    /// Made-up results (demo and screenshots).
    Mock(Scan),
    Real { finder: Finder, seen: u64, status: Status },
}

pub struct DupesUi {
    pub places: Vec<Place>,
    pub groups: Vec<Group>,
    pub rules: Vec<Rule>,
    pub prefer: Prefer,
    pub filter: Filter,
    pub sort: Sort,
    pub search: String,
    pub expanded: HashSet<u32>,
    /// (group id, copy index)
    pub selected: Option<(u32, usize)>,
    pub scroll: f32,
    pub content_h: f32,
    run: Run,
    /// The places changed since the results were found.
    pub stale: bool,
    /// Compare every byte of big files rather than samples.
    pub exact: bool,
    /// Rule being typed, and whether its field still needs the focus.
    pub rule_draft: Option<String>,
    pub draft_focus: bool,
    pub add_menu: Option<Pos2>,
    /// The menu of the remove button is open.
    pub trash_menu: bool,
    /// When each group showed up in the list (for its entrance).
    appeared: HashMap<u32, f64>,
    thumbs: Thumbs,
    /// Photo and video details, read on demand.
    meta: Arc<Mutex<HashMap<u32, Meta>>>,
    meta_asked: HashSet<u32>,
    /// Scroll the selection into view on the next frame.
    reveal: bool,
    /// Finder keys → group ids.
    ids: HashMap<u64, u32>,
    next_id: u32,
    last_poll: Instant,
    pub removing: Option<Arc<remove::Progress>>,
    /// Paths already removed: never shown again.
    gone: HashSet<String>,
    /// Servers already asked for a password.
    auth_asked: HashSet<String>,
    /// Bumped whenever groups or their selection change.
    generation: u64,
    view: View,
}

/// What the screen shows of the groups, worked out again only when
/// something changed (not on every frame: there can be tens of thousands).
#[derive(Default)]
struct View {
    key: Option<(u64, Filter, Sort, String, u32)>,
    /// Shown groups, in order.
    order: Vec<usize>,
    /// Over every group found (whatever the filter).
    freed: u64,
    extra: u64,
    marked: usize,
    unsafe_groups: usize,
    /// Groups per filter: all, photos, videos, other.
    counts: [usize; 4],
    any: bool,
}

impl DupesUi {
    pub fn new() -> DupesUi {
        DupesUi {
            places: Vec::new(),
            groups: Vec::new(),
            rules: Vec::new(),
            prefer: Prefer::Oldest,
            filter: Filter::All,
            sort: Sort::Size,
            search: String::new(),
            expanded: HashSet::new(),
            selected: None,
            scroll: 0.0,
            content_h: 0.0,
            run: Run::Idle,
            stale: false,
            exact: false,
            rule_draft: None,
            draft_focus: false,
            add_menu: None,
            trash_menu: false,
            appeared: HashMap::new(),
            thumbs: Thumbs::default(),
            meta: Default::default(),
            meta_asked: HashSet::new(),
            reveal: false,
            ids: HashMap::new(),
            next_id: 0,
            last_poll: Instant::now(),
            removing: None,
            gone: HashSet::new(),
            auth_asked: HashSet::new(),
            generation: 0,
            view: View::default(),
        }
    }

    /// Made-up places and results, found over a few seconds.
    pub fn mock() -> DupesUi {
        let mut d = DupesUi::new();
        d.places = mock::places();
        d.rules = mock::rules();
        d.groups = mock::groups(d.places.len());
        d.reapply();
        d.run = Run::Mock(Scan::new());
        d
    }

    fn is_mock(&self) -> bool {
        matches!(self.run, Run::Mock(_))
    }

    pub fn is_scanning(&self) -> bool {
        match &self.run {
            Run::Idle => false,
            Run::Mock(s) => !s.is_done(),
            Run::Real { finder, .. } => !finder.is_done(),
        }
    }

    /// Made-up results show up as the made-up scan advances.
    fn visible_frac(&self) -> f32 {
        match &self.run {
            Run::Mock(s) => s.fraction(),
            _ => 1.0,
        }
    }

    /// Starts (or starts again) comparing the places.
    pub fn start(&mut self) {
        self.appeared.clear();
        self.selected = None;
        self.scroll = 0.0;
        self.stale = false;
        if self.is_mock() {
            self.run = Run::Mock(Scan::new());
            return;
        }
        let specs: Vec<Spec> = self.places.iter().filter_map(|p| p.spec.clone()).collect();
        self.touch();
        self.run = if specs.is_empty() { Run::Idle } else { Run::Real { finder: Finder::start(specs, Options { exact: self.exact, ..Options::default() }), seen: 0, status: Status::default() } };
        self.groups.clear();
    }

    pub fn stop(&mut self) {
        if let Run::Real { finder, .. } = &self.run {
            finder.cancel();
            self.stale = true;
        }
    }

    fn status(&self) -> Option<&Status> {
        match &self.run {
            Run::Real { status, .. } => Some(status),
            _ => None,
        }
    }

    /// Picks up what the finder found since the last frame.
    fn poll(&mut self) {
        let mut found = None;
        if let Run::Real { finder, seen, status } = &mut self.run {
            *status = finder.status();
            if self.last_poll.elapsed() > Duration::from_millis(250) || status.done {
                self.last_poll = Instant::now();
                found = finder.changes_since(seen);
            }
        }
        if let Some(f) = found {
            self.merge(f);
        }
        self.thumbs.collect();
        // copies removed meanwhile
        let removed: Vec<String> = self.removing.as_ref().map(|r| std::mem::take(&mut *r.removed.lock().unwrap())).unwrap_or_default();
        if !removed.is_empty() {
            self.gone.extend(removed);
            let gone = &self.gone;
            for g in self.groups.iter_mut() {
                g.copies.retain(|c| !gone.contains(&c.path));
            }
            self.groups.retain(|g| g.copies.len() > 1);
            self.reapply();
            self.touch();
            if self.selected.is_some_and(|(id, c)| self.group_by_id(id).is_none_or(|gi| c >= self.groups[gi].copies.len())) {
                self.selected = None;
            }
        }
    }

    /// Takes in the size buckets the finder changed, keeping what the user
    /// chose by hand. Groups of other sizes are left as they are.
    fn merge(&mut self, changes: Vec<(u64, Vec<FoundGroup>)>) {
        if changes.is_empty() {
            return;
        }
        let sizes: HashSet<u64> = changes.iter().map(|(s, _)| *s).collect();
        let (old, keep): (Vec<Group>, Vec<Group>) = std::mem::take(&mut self.groups).into_iter().partition(|g| sizes.contains(&g.size));
        self.groups = keep;
        let prev: HashMap<u32, Group> = old.into_iter().map(|g| (g.id, g)).collect();
        let meta = self.meta.clone();
        let meta = meta.lock().unwrap();
        self.touch();
        for fg in changes.into_iter().flat_map(|(_, g)| g) {
            let files: Vec<_> = fg.files.into_iter().filter(|f| !self.gone.contains(&f.path)).collect();
            if files.len() < 2 {
                continue;
            }
            let next = &mut self.next_id;
            let id = *self.ids.entry(fg.key).or_insert_with(|| {
                *next += 1;
                *next
            });
            let old = prev.get(&id);
            let manual = old.is_some_and(|g| g.manual);
            let copies: Vec<Copy> = files
                .into_iter()
                .map(|f| {
                    let mut c = Copy::new(f.path, f.place, ymd(f.mtime));
                    c.shares = f.shares_data;
                    c.mtime = f.mtime;
                    c.marked = manual && old.and_then(|g| g.copies.iter().find(|o| o.path == c.path)).is_some_and(|o| o.marked);
                    c
                })
                .collect();
            let name = copies[0].name().to_string();
            let ext = name.rsplit_once('.').map(|(_, e)| e.to_uppercase()).unwrap_or_default();
            let mut g = Group {
                id,
                media: Media::of(&name),
                name,
                ext,
                size: fg.size,
                copies,
                keeper: 0,
                manual,
                meta: meta.get(&id).cloned().or_else(|| old.map(|g| g.meta.clone())).unwrap_or_default(),
                digest: fg.digest,
                seed: (fg.key ^ (fg.key >> 32)) as u32,
                found_at: 0.0,
            };
            apply_rules(&mut g, &self.rules, self.prefer);
            self.groups.push(g);
        }
    }

    fn reapply(&mut self) {
        for g in self.groups.iter_mut() {
            apply_rules(g, &self.rules, self.prefer);
        }
        self.touch();
    }

    /// The groups or their selection changed: the view is worked out again.
    fn touch(&mut self) {
        self.generation += 1;
    }

    /// Order of the list and totals, up to date.
    fn view(&mut self) -> &View {
        let frac = self.visible_frac();
        let key = (self.generation, self.filter, self.sort, self.search.clone(), (frac * 1000.0) as u32);
        if self.view.key.as_ref() != Some(&key) {
            let q = self.search.trim().to_lowercase();
            let g = &self.groups;
            let mut v = View { key: Some(key), ..Default::default() };
            for (i, x) in g.iter().enumerate() {
                if x.found_at > frac {
                    continue;
                }
                v.any = true;
                v.freed += x.freed();
                v.extra += x.extra();
                v.marked += x.marked();
                v.unsafe_groups += x.none_kept() as usize;
                for (k, f) in [Filter::All, Filter::Photos, Filter::Videos, Filter::Other].into_iter().enumerate() {
                    v.counts[k] += f.accepts(x.media) as usize;
                }
                let hit = self.filter.accepts(x.media) && (q.is_empty() || x.name.to_lowercase().contains(&q) || x.copies.iter().any(|c| c.path.to_lowercase().contains(&q)));
                if hit {
                    v.order.push(i);
                }
            }
            match self.sort {
                Sort::Size => v.order.sort_by(|a, b| g[*b].extra().cmp(&g[*a].extra()).then(g[*a].name.cmp(&g[*b].name))),
                Sort::Copies => v.order.sort_by(|a, b| g[*b].copies.len().cmp(&g[*a].copies.len()).then(g[*b].extra().cmp(&g[*a].extra()))),
                Sort::Name => v.order.sort_by_cached_key(|i| g[*i].name.to_lowercase()),
            }
            self.view = v;
        }
        &self.view
    }

    fn found(&self) -> impl Iterator<Item = &Group> {
        let f = self.visible_frac();
        self.groups.iter().filter(move |g| g.found_at <= f)
    }

    /// The groups shown, in order.
    fn order(&mut self) -> Vec<usize> {
        self.view().order.clone()
    }

    fn lines(&mut self) -> Vec<Line> {
        let mut out = Vec::new();
        let mut y = 0.0;
        for gi in self.order() {
            let g = &self.groups[gi];
            out.push(Line { group: gi, copy: None, y, h: GROUP_H });
            y += GROUP_H;
            if self.expanded.contains(&g.id) {
                for ci in 0..g.copies.len() {
                    out.push(Line { group: gi, copy: Some(ci), y, h: COPY_H });
                    y += COPY_H;
                }
                y += 4.0;
            }
            y += GROUP_GAP;
        }
        out
    }

    fn is_selected(&self, l: &Line) -> bool {
        let g = &self.groups[l.group];
        match (self.selected, l.copy) {
            (Some((id, _)), None) => id == g.id && !self.expanded.contains(&g.id),
            (Some((id, c)), Some(ci)) => id == g.id && c == ci,
            _ => false,
        }
    }

    fn select_line(&mut self, l: &Line) {
        let g = &self.groups[l.group];
        self.selected = Some((g.id, l.copy.unwrap_or(g.keeper)));
    }

    fn group_by_id(&self, id: u32) -> Option<usize> {
        self.groups.iter().position(|g| g.id == id)
    }

    /// Debug: skips the made-up scan and opens the biggest set.
    pub fn debug_pick(&mut self) {
        if let Run::Mock(s) = &mut self.run {
            s.started -= s.length + Duration::from_secs(1);
        }
        if let Some(gi) = self.order().first().copied() {
            let g = &self.groups[gi];
            self.expanded.insert(g.id);
            self.selected = Some((g.id, g.keeper));
        }
    }

    /// Checkbox of a group: clears its selection, or brings back the
    /// automatic one.
    fn toggle_group(&mut self, gi: usize) {
        let (rules, prefer) = (self.rules.clone(), self.prefer);
        let g = &mut self.groups[gi];
        if g.marked() > 0 {
            g.copies.iter_mut().for_each(|c| c.marked = false);
            g.manual = true;
        } else {
            g.manual = false;
            apply_rules(g, &rules, prefer);
        }
        self.touch();
    }

    fn toggle_copy(&mut self, gi: usize, ci: usize) {
        let g = &mut self.groups[gi];
        // files inside an app's library stay
        if g.copies[ci].locked {
            return;
        }
        g.copies[ci].marked = !g.copies[ci].marked;
        g.manual = true;
        self.touch();
    }

    fn remove_place(&mut self, k: usize) {
        self.places.remove(k);
        for g in self.groups.iter_mut() {
            g.copies.retain(|c| c.place != k);
            g.copies.iter_mut().filter(|c| c.place > k).for_each(|c| c.place -= 1);
        }
        self.groups.retain(|g| g.copies.len() > 1);
        self.reapply();
        if self.selected.is_some_and(|(id, _)| self.group_by_id(id).is_none()) {
            self.selected = None;
        }
        match self.run {
            Run::Mock(_) => self.start(),
            Run::Real { .. } => {
                self.stop();
                self.stale = !self.places.is_empty();
            }
            Run::Idle => {}
        }
    }

    pub fn add_place(&mut self, place: Place) {
        if self.places.iter().any(|p| p.spec.is_some() && p.spec == place.spec) {
            // the same place again (e.g. after typing a server's password): compare again
            if !self.is_scanning() {
                self.start();
            }
            return;
        }
        self.places.push(place);
        match self.run {
            Run::Mock(_) => self.start(),
            Run::Real { .. } => {
                self.stop();
                self.stale = true;
            }
            Run::Idle => {}
        }
    }

    /// Texture and aspect ratio of the group's preview, when there is one.
    fn thumb(&mut self, ctx: &egui::Context, gi: usize) -> Option<(egui::TextureId, f32)> {
        let g = &self.groups[gi];
        if !g.media.is_visual() {
            return None;
        }
        if self.is_mock() {
            let seed = g.seed;
            let t = self.thumbs.tex.entry(seed).or_insert_with(|| Some((ctx.load_texture(format!("dupe-{seed}"), fake_photo(seed, 400, 300), egui::TextureOptions::LINEAR), 4.0 / 3.0)));
            return t.as_ref().map(|(t, a)| (t.id(), *a));
        }
        // the preview of a copy on this computer
        let local = g.copies.iter().find(|c| matches!(self.places.get(c.place).and_then(|p| p.spec.as_ref()), Some(Spec::Local(_))))?;
        self.thumbs.get(ctx, g.id, &local.path)
    }

    /// Asks Spotlight about the selected photo or video.
    fn ask_meta(&mut self, gi: usize) {
        let g = &self.groups[gi];
        if self.is_mock() || !self.meta_asked.insert(g.id) {
            return;
        }
        let Some(c) = g.copies.iter().find(|c| matches!(self.places.get(c.place).and_then(|p| p.spec.as_ref()), Some(Spec::Local(_)))) else { return };
        let (id, path, meta) = (g.id, c.path.clone(), self.meta.clone());
        std::thread::spawn(move || {
            if let Some(m) = read_meta(&path) {
                meta.lock().unwrap().insert(id, m);
            }
        });
    }

    /// What a list row shows for a copy: the whole path of its folder, or
    /// its whole path when it has another name than the group.
    fn shown(c: &Copy, group_name: &str) -> String {
        if c.name() != group_name {
            return c.path.clone();
        }
        match c.path.rsplit_once(['/', '\\']) {
            Some((dir, _)) if !dir.is_empty() => dir.to_string(),
            _ => c.path.clone(),
        }
    }

    fn meta_of(&self, gi: usize) -> Meta {
        let g = &self.groups[gi];
        self.meta.lock().unwrap().get(&g.id).cloned().unwrap_or_else(|| g.meta.clone())
    }

    /// What the selected copies amount to, place by place.
    fn removal(&self) -> Vec<remove::Batch> {
        let at = |c: &Copy| self.places.get(c.place).and_then(|p| p.spec.as_ref()).map(remove::Location::of);
        let mut out = Vec::new();
        for g in self.found() {
            if g.marked() == 0 || g.none_kept() {
                continue;
            }
            let target = |c: &Copy| at(c).map(|a| remove::Target { at: a, path: c.path.clone(), frees: !c.shares, mtime: (c.mtime != 0).then_some(c.mtime) });
            let remove: Vec<_> = g.copies.iter().filter(|c| c.marked && !c.kept()).filter_map(target).collect();
            let keep: Vec<_> = g.copies.iter().filter(|c| !c.marked).filter_map(target).collect();
            if !remove.is_empty() {
                out.push(remove::Batch { size: g.size, remove, keep });
            }
        }
        out
    }
}

/// The place a scan target becomes on this screen.
pub fn place_for(t: &Target) -> Place {
    match t {
        Target::Volume(v) => Place { kind: PlaceKind::Disk, label: v.name.clone(), path: v.mount.clone(), spec: Some(Spec::Local(v.mount.clone().into())) },
        Target::Folder(p) => Place {
            kind: PlaceKind::Folder,
            label: t.title(),
            path: p.to_string_lossy().into_owned(),
            spec: Some(Spec::Local(p.clone())),
        },
        Target::Ssh { host, path } => Place { kind: PlaceKind::Server, label: t.title(), path: format!("{host}:{path}"), spec: Some(Spec::Ssh { host: host.clone(), path: path.clone() }) },
        Target::Rclone(r) => Place { kind: PlaceKind::Cloud, label: r.clone(), path: r.clone(), spec: Some(Spec::Rclone(r.clone())) },
    }
}

/// macOS: dimensions, duration, camera and date from Spotlight.
fn read_meta(path: &str) -> Option<Meta> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let names = ["kMDItemPixelWidth", "kMDItemPixelHeight", "kMDItemDurationSeconds", "kMDItemAcquisitionModel", "kMDItemContentCreationDate"];
    let mut c = std::process::Command::new("mdls");
    c.arg("-raw");
    for n in names {
        c.args(["-name", n]);
    }
    let out = c.arg(path).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let v: Vec<&str> = text.split('\0').map(|s| s.trim()).collect();
    let get = |k: usize| v.get(k).copied().filter(|s| !s.is_empty() && *s != "(null)");
    let num = |k: usize| get(k).and_then(|s| s.parse::<f64>().ok());
    let date = get(4).and_then(|s| Some((s.get(0..4)?.parse().ok()?, s.get(5..7)?.parse().ok()?, s.get(8..10)?.parse().ok()?)));
    Some(Meta {
        dims: num(0).zip(num(1)).map(|(w, h)| (w as u32, h as u32)),
        duration: num(2).map(|s| s.round() as u32),
        device: get(3).map(|s| s.trim_matches('"').to_string()),
        taken: date,
    })
}

/// Previews of files on this computer, made by Quick Look in the
/// background (it reads HEIC, RAW and videos alike).
#[derive(Default)]
struct Thumbs {
    /// Keyed by group id (or seed for made-up ones); None: no preview.
    tex: HashMap<u32, Option<(egui::TextureHandle, f32)>>,
    asked: HashSet<u32>,
    shared: Arc<ThumbQueue>,
    started: bool,
}

#[derive(Default)]
struct ThumbQueue {
    todo: Mutex<Vec<(u32, String)>>,
    cv: Condvar,
    done: Mutex<Vec<(u32, Option<egui::ColorImage>)>>,
    ctx: Mutex<Option<egui::Context>>,
}

impl Thumbs {
    fn get(&mut self, ctx: &egui::Context, id: u32, path: &str) -> Option<(egui::TextureId, f32)> {
        if let Some(t) = self.tex.get(&id) {
            return t.as_ref().map(|(t, a)| (t.id(), *a));
        }
        if !cfg!(target_os = "macos") || !self.asked.insert(id) {
            return None;
        }
        if !self.started {
            self.started = true;
            *self.shared.ctx.lock().unwrap() = Some(ctx.clone());
            for _ in 0..2 {
                let q = self.shared.clone();
                std::thread::spawn(move || thumb_worker(q));
            }
        }
        self.shared.todo.lock().unwrap().push((id, path.to_string()));
        self.shared.cv.notify_one();
        None
    }

    /// Turns finished previews into textures.
    fn collect(&mut self) {
        let done: Vec<_> = std::mem::take(&mut *self.shared.done.lock().unwrap());
        if done.is_empty() {
            return;
        }
        let ctx = self.shared.ctx.lock().unwrap().clone();
        for (id, img) in done {
            let tex = match (img, &ctx) {
                (Some(img), Some(ctx)) => {
                    let aspect = img.size[0] as f32 / img.size[1].max(1) as f32;
                    Some((ctx.load_texture(format!("dupe-thumb-{id}"), img, egui::TextureOptions::LINEAR), aspect))
                }
                _ => None,
            };
            self.tex.insert(id, tex);
        }
    }
}

fn thumb_worker(q: Arc<ThumbQueue>) {
    loop {
        let (id, path) = {
            let mut todo = q.todo.lock().unwrap();
            loop {
                // newest first: what is on screen now
                if let Some(j) = todo.pop() {
                    break j;
                }
                todo = q.cv.wait(todo).unwrap();
            }
        };
        let img = quicklook(&path, id);
        q.done.lock().unwrap().push((id, img));
        if let Some(ctx) = q.ctx.lock().unwrap().as_ref() {
            ctx.request_repaint();
        }
    }
}

fn quicklook(path: &str, id: u32) -> Option<egui::ColorImage> {
    let dir = std::env::temp_dir().join(format!("squirreldisk-thumbs-{}", std::process::id())).join(id.to_string());
    std::fs::create_dir_all(&dir).ok()?;
    let status = std::process::Command::new("qlmanage")
        .args(["-t", "-s", "480", "-o"])
        .arg(&dir)
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    let name = std::path::Path::new(path).file_name()?.to_string_lossy().into_owned();
    let png = dir.join(format!("{name}.png"));
    let img = status.success().then(|| tiny_skia::Pixmap::load_png(&png).ok()).flatten();
    let _ = std::fs::remove_dir_all(&dir);
    let pm = img?;
    Some(egui::ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data()))
}

impl App {
    pub fn open_dupes(&mut self) {
        if self.dupes.is_none() {
            let mock = crate::scan::demo::enabled() || std::env::var_os("SQD_DUPES_MOCK").is_some();
            self.dupes = Some(if mock { DupesUi::mock() } else { DupesUi::new() });
        }
        self.screen = Screen::Dupes;
        self.sfx(Sfx::Blip);
    }

    /// Esc on the duplicates screen: closes what is open on it first.
    pub fn dupes_escape(&mut self) {
        match self.dupes.as_mut() {
            Some(d) if d.add_menu.is_some() => d.add_menu = None,
            Some(d) if d.trash_menu => d.trash_menu = false,
            Some(d) if d.rule_draft.is_some() => d.rule_draft = None,
            _ => self.screen = Screen::Home,
        }
    }

    pub fn dupes_ui(&mut self, ui: &mut Ui, screen: Rect) {
        let Some(mut d) = self.dupes.take() else {
            self.screen = Screen::Home;
            return;
        };
        let theme = self.theme.clone();
        let was_scanning = d.is_scanning();
        d.poll();
        if was_scanning && !d.is_scanning() && !d.stale {
            self.sfx(Sfx::Success);
        }
        // a server that wants a password: ask once
        if self.modal.is_none() {
            let ask = d.status().and_then(|st| {
                st.places.iter().enumerate().find_map(|(k, ps)| match (d.places.get(k).and_then(|p| p.spec.as_ref()), &ps.error) {
                    (Some(Spec::Ssh { host, path }), Some(e)) if crate::scan::remote::is_auth_error(e) && !d.auth_asked.contains(host) => Some((host.clone(), path.clone())),
                    _ => None,
                })
            });
            if let Some((host, path)) = ask {
                d.auth_asked.insert(host.clone());
                let retry = self.ssh_tried.contains(&host);
                self.ssh_tried.insert(host.clone());
                self.open_modal(Modal::SshAuth { host, path, secret: String::new(), retry });
            }
        }
        let bar_h = self.chrome.0 * 2.0;
        let bar = Rect::from_min_size(screen.min, Vec2::new(screen.width(), bar_h));
        self.window_drag_area(ui, Rect::from_min_size(screen.min, Vec2::new(screen.width(), bar_h + 4.0)));
        self.dupes_top_bar(ui, bar);

        let body = Rect::from_min_max(Pos2::new(screen.left() + 16.0, bar.bottom() + 6.0), Pos2::new(screen.right() - 16.0, screen.bottom() - 16.0));
        let panel_w = (body.width() * 0.34).clamp(300.0, 420.0);
        let panel = Rect::from_min_max(Pos2::new(body.right() - panel_w, body.top()), body.max);
        let left = Rect::from_min_max(body.min, Pos2::new(panel.left() - 16.0, body.bottom()));

        let places = Rect::from_min_size(left.min, Vec2::new(left.width(), 108.0));
        self.dupes_places(ui, places, &mut d);
        let tools = Rect::from_min_size(Pos2::new(left.left(), places.bottom() + 14.0), Vec2::new(left.width(), 34.0));
        dupes_toolbar(ui, tools, &mut d, &theme);
        let rules_top = tools.bottom() + 10.0;
        let rules_h = dupes_rules(ui, Rect::from_min_size(Pos2::new(left.left(), rules_top), Vec2::new(left.width(), 30.0)), &mut d, &theme);
        let actions = Rect::from_min_max(Pos2::new(left.left(), left.bottom() - 58.0), left.max);
        let list = Rect::from_min_max(Pos2::new(left.left(), rules_top + rules_h + 10.0), Pos2::new(left.right(), actions.top() - 10.0));
        self.dupes_list(ui, list, &mut d);
        self.dupes_actions(ui, actions, &mut d);
        dupes_preview(ui, panel, &mut d, &theme);
        self.dupes_add_menu(ui, &mut d);
        self.dupes = Some(d);
    }

    fn dupes_top_bar(&mut self, ui: &mut Ui, bar: Rect) {
        let theme = self.theme.clone();
        let mut x = bar.left() + if cfg!(target_os = "macos") { self.chrome.1 + 18.0 } else { 14.0 };
        let cy = bar.center().y;
        let nav = 24.0;
        if widgets::icon_button(ui, Rect::from_center_size(Pos2::new(x + nav / 2.0, cy), Vec2::splat(nav)), Id::new("dupes-back"), Icon::Back, &theme, true).clicked() {
            self.screen = Screen::Home;
            self.sfx(Sfx::BlipDown);
        }
        x += nav + 12.0;
        let home = tr("Home");
        let tw = ui.painter().layout_no_wrap(home.to_string(), font(13.0), Color32::WHITE).size().x;
        let r = Rect::from_min_size(Pos2::new(x, cy - 11.0), Vec2::new(tw + 12.0, 22.0));
        let resp = ui.interact(r, Id::new("dupes-crumb"), Sense::click());
        let h = ui.ctx().animate_bool_with_time(Id::new("dupes-crumb-h"), resp.hovered(), 0.1);
        let p = ui.painter();
        p.rect_filled(r, cr(5.0), with_alpha(theme.surface_hi, 0.7 * h));
        p.text(Pos2::new(r.left() + 6.0, cy), Align2::LEFT_CENTER, home, font(13.0), lerp_color(theme.text_dim, theme.text, h));
        p.text(Pos2::new(r.right() + 4.0, cy), Align2::CENTER_CENTER, "/", font(13.0), theme.text_faint);
        p.text(Pos2::new(r.right() + 14.0, cy), Align2::LEFT_CENTER, tr("Duplicates"), bold(13.0), theme.text);
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            self.screen = Screen::Home;
            self.sfx(Sfx::BlipDown);
        }
        let gear = Rect::from_center_size(Pos2::new(bar.right() - 24.0, cy), Vec2::splat(nav));
        if widgets::icon_button(ui, gear, Id::new("dupes-settings"), Icon::Gear, &theme, true).clicked() {
            self.open_modal(super::app::Modal::Settings);
        }
    }

    /// The places compared, the scan's progress and the space to win back.
    fn dupes_places(&mut self, ui: &mut Ui, r: Rect, d: &mut DupesUi) {
        let theme = self.theme.clone();
        card(ui.painter(), r, &theme);
        let inner = r.shrink2(Vec2::new(18.0, 14.0));
        let right_w = 190.0;
        let left = Rect::from_min_max(inner.min, Pos2::new(inner.right() - right_w - 24.0, inner.bottom()));

        // space to win back
        let (freed, extra) = (d.view().freed, d.view().extra);
        let p = ui.painter();
        p.text(Pos2::new(inner.right(), inner.top() + 2.0), Align2::RIGHT_TOP, tr("CAN BE FREED"), bold(11.0), theme.text_faint);
        let shown = ui.ctx().animate_value_with_time(Id::new("dupes-freed"), freed as f32, 0.35);
        p.text(Pos2::new(inner.right(), inner.top() + 18.0), Align2::RIGHT_TOP, fmt_size(shown as u64), display(30.0), theme.text);
        p.text(Pos2::new(inner.right(), inner.top() + 58.0), Align2::RIGHT_TOP, trf("of {size} in extra copies", &[("size", &fmt_size(extra))]), font(12.0), theme.text_dim);
        p.line_segment([Pos2::new(left.right() + 12.0, inner.top() + 2.0), Pos2::new(left.right() + 12.0, inner.bottom() - 2.0)], Stroke::new(1.0, with_alpha(theme.stroke, 0.6)));

        // places
        p.text(left.left_top(), Align2::LEFT_TOP, tr("COMPARING"), bold(11.0), theme.text_faint);
        // how thorough: samples (fast) or every byte
        {
            let label = tr("Exact comparison");
            let lw = p.layout_no_wrap(label.to_string(), font(11.5), Color32::WHITE).size().x;
            let track = Rect::from_min_size(Pos2::new(left.right() - 26.0, left.top() - 1.0), Vec2::new(26.0, 15.0));
            let hit = Rect::from_min_max(Pos2::new(track.left() - lw - 8.0, track.top() - 3.0), track.right_bottom() + Vec2::new(0.0, 3.0));
            let id = Id::new("dupes-exact");
            let resp = ui.interact(hit, id, Sense::click()).on_hover_text(tr(
                "Off: compares the start, the end and dozens of spots spread over each big file, which tells photos and videos apart in practice. On: reads every byte, certain but much slower on big videos and slow cards.",
            ));
            if resp.clicked() {
                d.exact = !d.exact;
                if !matches!(d.run, Run::Idle | Run::Mock(_)) {
                    d.stop();
                    d.stale = true;
                }
            }
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let t = ui.ctx().animate_bool_with_time(id, d.exact, 0.15);
            let h = ui.ctx().animate_bool_with_time(id.with("h"), resp.hovered(), 0.1);
            let p = ui.painter();
            p.text(Pos2::new(track.left() - 7.0, track.center().y), Align2::RIGHT_CENTER, label, font(11.5), lerp_color(theme.text_faint, theme.text, t.max(h * 0.6)));
            p.rect_filled(track, cr(7.5), lerp_color(theme.surface_hi, theme.accent, t));
            p.circle_filled(Pos2::new(track.left() + 7.5 + t * 11.0, track.center().y), 5.5, Color32::WHITE);
        }
        let mut x = left.left();
        let chip_y = left.top() + 20.0;
        let chip_w = |p: &Painter, label: &str| p.layout_no_wrap(label.to_string(), bold(12.5), Color32::WHITE).size().x + 52.0;
        let labels: Vec<String> = d.places.iter().map(|pl| widgets::truncate(ui.painter(), &pl.label, &bold(12.5), 140.0)).collect();
        let all_w: f32 = labels.iter().map(|l| chip_w(ui.painter(), l) + 8.0).sum();
        // "Add place" in words when there is room, else just a +
        let wide_add = all_w + 108.0 <= left.width() - 30.0;
        let add_w = if wide_add { 100.0 } else { 28.0 };
        let mut remove = None;
        for (k, place) in d.places.iter().enumerate() {
            let label = &labels[k];
            let rr = Rect::from_min_size(Pos2::new(x, chip_y), Vec2::new(chip_w(ui.painter(), label), 28.0));
            let rest_w = if k + 1 == d.places.len() { 0.0 } else { 70.0 };
            if rr.right() + rest_w > left.right() - add_w - 38.0 && k > 0 {
                // no room: say how many more
                let more = trf("+{n} more", &[("n", &(d.places.len() - k))]);
                ui.painter().text(Pos2::new(x + 4.0, rr.center().y), Align2::LEFT_CENTER, more, font(12.0), theme.text_dim);
                x += 64.0;
                break;
            }
            let id = Id::new(("dupe-place", k));
            let error = d.status().and_then(|st| st.places.get(k)).and_then(|ps| ps.error.clone()).filter(|e| e != "cancelled");
            let tip = match &error {
                Some(e) => format!("{}\n{e}", place.path),
                None => place.path.clone(),
            };
            let resp = ui.interact(rr, id, Sense::hover()).on_hover_text(tip);
            let h = ui.ctx().animate_bool_with_time(id, resp.hovered(), 0.12);
            let p = ui.painter();
            p.rect_filled(rr, cr(14.0), lerp_color(with_alpha(theme.bg_bottom, 0.5), theme.surface_hi, h));
            let edge = if error.is_some() { with_alpha(theme.danger, 0.8) } else { with_alpha(theme.stroke, 0.7) };
            p.rect_stroke(rr, cr(14.0), Stroke::new(1.0, edge), egui::StrokeKind::Inside);
            let (icon, col) = if error.is_some() { (Icon::Warning, theme.danger) } else { (place_icon(place.kind), place_color(k, &theme)) };
            widgets::draw_icon(p, icon, Rect::from_center_size(Pos2::new(rr.left() + 16.0, rr.center().y), Vec2::splat(14.0)), col);
            p.text(Pos2::new(rr.left() + 30.0, rr.center().y), Align2::LEFT_CENTER, label, bold(12.5), theme.text);
            let xr = Rect::from_center_size(Pos2::new(rr.right() - 13.0, rr.center().y), Vec2::splat(18.0));
            let xr_resp = ui.interact(xr, id.with("x"), Sense::click());
            let xh = ui.ctx().animate_bool_with_time(id.with("xh"), xr_resp.hovered(), 0.1);
            let p = ui.painter();
            p.circle_filled(xr.center(), 9.0, with_alpha(theme.surface_hi, xh));
            widgets::draw_icon(p, Icon::Close, Rect::from_center_size(xr.center(), Vec2::splat(7.0)), lerp_color(theme.text_faint, theme.text, xh.max(h * 0.5)));
            if xr_resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if xr_resp.clicked() {
                remove = Some(k);
            }
            x = rr.right() + 8.0;
        }
        let add = Rect::from_min_size(Pos2::new(x, chip_y), Vec2::new(add_w, 28.0));
        let mut resp = ui.interact(add, Id::new("dupe-add-place"), Sense::click());
        if !wide_add {
            resp = resp.on_hover_text(tr("Add place"));
        }
        let h = ui.ctx().animate_bool_with_time(Id::new("dupe-add-place-h"), resp.hovered() || d.add_menu.is_some(), 0.12);
        let p = ui.painter();
        p.rect_stroke(add, cr(14.0), Stroke::new(1.0, lerp_color(with_alpha(theme.stroke, 0.9), theme.accent, h)), egui::StrokeKind::Inside);
        p.rect_filled(add, cr(14.0), with_alpha(theme.accent, 0.12 * h));
        let plus_x = if wide_add { add.left() + 16.0 } else { add.center().x };
        widgets::draw_icon(p, Icon::Plus, Rect::from_center_size(Pos2::new(plus_x, add.center().y), Vec2::splat(11.0)), lerp_color(theme.text_dim, theme.accent, h));
        if wide_add {
            p.text(Pos2::new(add.left() + 28.0, add.center().y), Align2::LEFT_CENTER, tr("Add place"), bold(12.5), lerp_color(theme.text_dim, theme.text, h));
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            d.add_menu = if d.add_menu.is_some() { None } else { Some(add.left_bottom() + Vec2::new(0.0, 6.0)) };
        }
        if d.places.is_empty() {
            ui.painter().text(Pos2::new(add.right() + 12.0, add.center().y), Align2::LEFT_CENTER, tr("Folders, disks, servers or cloud storage"), font(12.0), theme.text_faint);
        }
        if let Some(k) = remove {
            d.remove_place(k);
            self.sfx(Sfx::BlipDown);
        }

        // progress, or the button that starts
        let line_y = left.top() + 63.0;
        let groups = d.groups.len() as u64;
        let (status, frac, done): (String, Option<f32>, bool) = match (&d.run, d.status()) {
            (Run::Mock(scan), _) => {
                let frac = scan.fraction();
                const FILES: u64 = 184_302;
                const BYTES: u64 = 1_420_000_000_000;
                let walk = (frac / 0.3).min(1.0);
                let text = if frac >= 1.0 {
                    trf("Done · {files} files compared, {groups} sets of duplicates", &[("files", &fmt_count(FILES)), ("groups", &fmt_count(groups))])
                } else if walk < 1.0 {
                    trf("Listing files · {files} files · {size}", &[("files", &fmt_count_compact((FILES as f32 * walk) as u64)), ("size", &fmt_size((BYTES as f32 * walk) as u64))])
                } else {
                    trf("Comparing {n} files of the same size · {read} of {total}", &[("n", &fmt_count(2387)), ("read", &fmt_size((BYTES as f32 * 0.04 * (frac - 0.3) / 0.7) as u64)), ("total", &fmt_size((BYTES as f32 * 0.04) as u64))])
                };
                (text, Some(frac), frac >= 1.0)
            }
            (Run::Real { .. }, Some(st)) if !d.stale => {
                let text = if st.done {
                    let mut t = trf("Done in {time} · {files} files, {groups} sets of duplicates", &[("time", &fmt_elapsed(st.elapsed)), ("files", &fmt_count(st.files)), ("groups", &fmt_count(groups))]);
                    if st.errors > 0 {
                        t += &trf(" · {n} could not be read", &[("n", &fmt_count(st.errors))]);
                    }
                    t
                } else if st.walking {
                    trf("Listing files · {files} files · {size}", &[("files", &fmt_count_compact(st.files)), ("size", &fmt_size(st.bytes))])
                } else {
                    trf("Comparing {n} files of the same size · {read} of {total}", &[("n", &fmt_count(st.candidates)), ("read", &fmt_size(st.read)), ("total", &fmt_size(st.to_read))])
                };
                (text, st.fraction(), st.done)
            }
            _ => (String::new(), None, false),
        };
        let idle = status.is_empty();
        let p = ui.painter();
        if idle {
            // not started, or the places changed since
            let hint = if d.places.is_empty() {
                tr("Add the places to compare.")
            } else if d.stale {
                tr("The places changed: compare again to update the results.")
            } else {
                tr("Ready when you are.")
            };
            let btn = Rect::from_min_size(Pos2::new(left.right() - 168.0, line_y - 10.0), Vec2::new(168.0, 32.0));
            let st = widgets::truncate(p, hint, &font(12.0), btn.left() - left.left() - 12.0);
            p.text(Pos2::new(left.left(), line_y + 6.0), Align2::LEFT_CENTER, st, font(12.0), theme.text_dim);
            let label = if d.stale { tr("Compare again") } else { tr("Find duplicates") };
            if widgets::button_ex(ui, btn, Id::new("dupes-start"), label, Some(Icon::Search), BtnStyle::Primary, &theme, !d.places.is_empty()).clicked() {
                d.start();
                self.sfx(Sfx::Blip);
            }
            return;
        }
        let action = Rect::from_center_size(Pos2::new(left.right() - 12.0, line_y + 6.0), Vec2::splat(24.0));
        let bar = Rect::from_min_size(Pos2::new(left.left(), line_y + 13.0), Vec2::new(left.width() - 34.0, 5.0));
        if !done {
            p.circle_filled(Pos2::new(left.left() + 4.0, line_y), 3.5, with_alpha(theme.accent2, 0.6 + 0.4 * ((self.time * 4.0).sin() as f32 * 0.5 + 0.5)));
        } else {
            widgets::draw_icon(p, Icon::Check, Rect::from_center_size(Pos2::new(left.left() + 5.0, line_y), Vec2::splat(11.0)), theme.ok);
        }
        let st = widgets::truncate(p, &status, &font(12.0), bar.width() - 50.0);
        p.text(Pos2::new(left.left() + 16.0, line_y), Align2::LEFT_CENTER, st, font(12.0), theme.text_dim);
        if let (false, Some(f)) = (done, frac) {
            p.text(Pos2::new(bar.right(), line_y), Align2::RIGHT_CENTER, format!("{:.0}%", f * 100.0), bold(12.0), theme.text);
        }
        widgets::progress_bar(p, bar, frac, &theme, self.time, if done { Some(theme.ok) } else { None });
        if done {
            if widgets::icon_button(ui, action, Id::new("dupes-rescan"), Icon::Refresh, &theme, true).on_hover_text(tr("Compare again")).clicked() {
                d.start();
                self.sfx(Sfx::Blip);
            }
        } else if widgets::icon_button(ui, action, Id::new("dupes-stop"), Icon::Close, &theme, !d.is_mock()).on_hover_text(tr("Stop")).clicked() {
            d.stop();
            self.sfx(Sfx::BlipDown);
        }
    }

    fn dupes_add_menu(&mut self, ui: &mut Ui, d: &mut DupesUi) {
        let Some(origin) = d.add_menu else { return };
        let theme = self.theme.clone();
        enum Pick {
            Folder,
            Volume(crate::disks::Volume),
            Server,
            Cloud,
        }
        let mut items: Vec<(Icon, String, String, Pick)> = vec![(Icon::Folder, tr("Folders…").into(), tr("Pick one or more").into(), Pick::Folder)];
        for v in &self.volumes {
            let sub = format!("{} · {}", fmt_size(v.total), v.mount);
            items.push((if v.removable { Icon::External } else { Icon::Disk }, v.name.clone(), sub, Pick::Volume(v.clone())));
        }
        items.push((Icon::Server, tr("Remote server").into(), tr("Over SSH").into(), Pick::Server));
        items.push((Icon::Cloud, tr("Cloud storage").into(), tr("Drive, S3, Dropbox… with rclone").into(), Pick::Cloud));
        let w = 280.0;
        let h = items.len() as f32 * 46.0 + 12.0;
        let mut chosen = None;
        let mut rect_out = Rect::NOTHING;
        egui::Area::new(Id::new("dupes-add-menu")).order(egui::Order::Foreground).fixed_pos(origin).show(ui.ctx(), |ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
            rect_out = rect;
            let p = ui.painter();
            widgets::shadow(p, rect, 12.0, 1.0, &theme);
            p.rect_filled(rect, cr(12.0), theme.surface_hi);
            p.rect_stroke(rect, cr(12.0), Stroke::new(1.0, theme.stroke), egui::StrokeKind::Inside);
            for (k, (icon, title, sub, _)) in items.iter().enumerate() {
                let rr = Rect::from_min_size(rect.left_top() + Vec2::new(6.0, 6.0 + k as f32 * 46.0), Vec2::new(w - 12.0, 44.0));
                let id = Id::new(("dupes-add", k));
                let resp = ui.interact(rr, id, Sense::click());
                let hh = ui.ctx().animate_bool_with_time(id, resp.hovered(), 0.08);
                let p = ui.painter();
                p.rect_filled(rr, cr(8.0), with_alpha(theme.accent, 0.85 * hh));
                widgets::draw_icon(p, *icon, Rect::from_center_size(Pos2::new(rr.left() + 20.0, rr.center().y), Vec2::splat(16.0)), lerp_color(theme.accent, Color32::WHITE, hh));
                let t = widgets::truncate(p, title, &bold(13.0), rr.width() - 52.0);
                p.text(Pos2::new(rr.left() + 42.0, rr.top() + 14.0), Align2::LEFT_CENTER, t, bold(13.0), lerp_color(theme.text, Color32::WHITE, hh));
                let st = widgets::truncate(p, sub, &font(11.5), rr.width() - 52.0);
                p.text(Pos2::new(rr.left() + 42.0, rr.top() + 31.0), Align2::LEFT_CENTER, st, font(11.5), lerp_color(theme.text_dim, Color32::WHITE, hh));
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    chosen = Some(k);
                }
            }
        });
        let clicked_out = ui.input(|i| i.pointer.any_pressed()) && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| !rect_out.contains(p));
        if let Some(k) = chosen {
            d.add_menu = None;
            match &items[k].3 {
                Pick::Folder => {
                    for p in rfd::FileDialog::new().set_title(tr("Choose folders to compare")).pick_folders().unwrap_or_default() {
                        d.add_place(place_for(&Target::Folder(p)));
                    }
                }
                Pick::Volume(v) => d.add_place(place_for(&Target::Volume(v.clone()))),
                // the usual dialogs: what they open comes back through
                // `start_session`, which adds it here (see `App::start_session`)
                Pick::Server => {
                    let host = self.settings.ssh_history.first().cloned().unwrap_or_default();
                    self.open_modal(Modal::Ssh { host, path: "/".into() });
                }
                Pick::Cloud => self.open_modal(Modal::Rclone { path: String::new(), confirm_remove: None }),
            }
            self.sfx(Sfx::Blip);
        } else if clicked_out {
            d.add_menu = None;
        }
    }

    fn dupes_list(&mut self, ui: &mut Ui, list: Rect, d: &mut DupesUi) {
        let theme = self.theme.clone();
        let ctx = ui.ctx().clone();
        let lines = d.lines();
        d.content_h = lines.last().map(|l| l.y + l.h + GROUP_GAP).unwrap_or(0.0);

        // keyboard: ↑↓ move, → open, ← close, space checks
        if self.modal.is_none() && !ctx.egui_wants_keyboard_input() && !lines.is_empty() {
            let (up, down, open, close, space) = ui.input(|i| {
                (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::ArrowRight), i.key_pressed(egui::Key::ArrowLeft), i.key_pressed(egui::Key::Space))
            });
            let cur = lines.iter().position(|l| d.is_selected(l));
            if up || down {
                let next = match cur {
                    None => 0,
                    Some(c) if down => (c + 1).min(lines.len() - 1),
                    Some(c) => c.saturating_sub(1),
                };
                d.select_line(&lines[next]);
                d.reveal = true;
            }
            if let Some(c) = cur {
                let l = lines[c];
                let gid = d.groups[l.group].id;
                if open && l.copy.is_none() {
                    d.expanded.insert(gid);
                }
                if close {
                    d.expanded.remove(&gid);
                    d.selected = Some((gid, d.groups[l.group].keeper));
                }
                if space {
                    match l.copy {
                        Some(ci) => d.toggle_copy(l.group, ci),
                        None => d.toggle_group(l.group),
                    }
                }
            }
        }

        // scrolling
        let max_scroll = (d.content_h - list.height()).max(0.0);
        if self.modal.is_none() && d.add_menu.is_none() && ui.rect_contains_pointer(list) {
            d.scroll -= ui.input(|i| i.smooth_scroll_delta.y);
        }
        if d.reveal {
            d.reveal = false;
            if let Some(l) = lines.iter().find(|l| d.is_selected(l)) {
                if l.y < d.scroll {
                    d.scroll = l.y;
                } else if l.y + l.h > d.scroll + list.height() {
                    d.scroll = l.y + l.h - list.height();
                }
            }
        }
        d.scroll = d.scroll.clamp(0.0, max_scroll);

        let lp = ui.painter().with_clip_rect(list.intersect(ui.clip_rect()));
        if lines.is_empty() {
            let msg = if d.view().any {
                tr("Nothing matches the search")
            } else if d.is_scanning() {
                tr("Looking for files with the same content…")
            } else if matches!(d.run, Run::Idle) || d.stale {
                tr("Duplicates will show up here as they are found.")
            } else {
                tr("No duplicates found")
            };
            lp.text(list.center() - Vec2::new(0.0, 20.0), Align2::CENTER_CENTER, msg, font(13.5), theme.text_dim);
            return;
        }
        let top = list.top() - d.scroll;
        let now = self.time;
        let mut click: Option<(Line, u8)> = None; // 0 row, 1 checkbox
        for l in lines.iter() {
            let y = top + l.y;
            if y > list.bottom() || y + l.h + 8.0 < list.top() {
                continue;
            }
            let g = &d.groups[l.group];
            // rows found while comparing slide in; the rest are just there
            let fresh = d.is_scanning();
            let t0 = *d.appeared.entry(g.id).or_insert(if fresh { now } else { now - 1.0 });
            let appear = ((now - t0) / 0.45).clamp(0.0, 1.0) as f32;
            let slide = (1.0 - super::sunburst::ease(appear)) * 18.0;
            match l.copy {
                None => {
                    let expanded = d.expanded.contains(&g.id);
                    if expanded {
                        let block_h = GROUP_H + g.copies.len() as f32 * COPY_H + 4.0;
                        let block = Rect::from_min_size(Pos2::new(list.left(), y), Vec2::new(list.width() - 10.0, block_h));
                        lp.rect_filled(block, cr(10.0), with_alpha(theme.surface, if theme.dark { 0.55 } else { 0.8 }));
                        lp.rect_stroke(block, cr(10.0), Stroke::new(1.0, with_alpha(theme.stroke, 0.5)), egui::StrokeKind::Inside);
                    }
                    let rr = Rect::from_min_size(Pos2::new(list.left() + slide, y), Vec2::new(list.width() - 10.0, GROUP_H));
                    let id = Id::new(("dupe-g", g.id));
                    let hit = rr.intersect(list);
                    let resp = ui.interact(hit, id, Sense::click());
                    let sel = d.is_selected(l);
                    let h = ctx.animate_bool_with_time(id.with("h"), resp.hovered() || sel, 0.1);
                    if resp.hovered() {
                        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if !expanded || h > 0.0 {
                        lp.rect_filled(rr, cr(10.0), with_alpha(if sel { lerp_color(theme.surface_hi, theme.accent, 0.18) } else { theme.surface_hi }, h * if expanded { 0.6 } else { 1.0 }));
                    }
                    if appear < 1.0 {
                        lp.rect_filled(rr, cr(10.0), with_alpha(theme.accent, 0.22 * (1.0 - appear)));
                    }
                    let a = appear;
                    // checkbox
                    let cb = Rect::from_center_size(Pos2::new(rr.left() + 20.0, rr.center().y), Vec2::splat(18.0));
                    let state = if g.marked() == 0 { 0 } else if g.marked() + 1 >= g.copies.len() || g.none_kept() { 2 } else { 1 };
                    let cb_resp = ui.interact(cb.expand(4.0).intersect(list), id.with("cb"), Sense::click());
                    draw_checkbox(&lp, cb, state, cb_resp.hovered(), &theme, a);
                    // chevron
                    let chev = Rect::from_center_size(Pos2::new(rr.left() + 44.0, rr.center().y), Vec2::splat(11.0));
                    widgets::draw_icon(&lp, if expanded { Icon::Expand } else { Icon::Forward }, chev, with_alpha(theme.text_faint, a));
                    // thumbnail
                    let thumb = Rect::from_min_size(Pos2::new(rr.left() + 58.0, rr.center().y - 18.0), Vec2::splat(36.0));
                    let (media, seed, ext) = (g.media, g.seed, g.ext.clone());
                    let tex = d.thumb(&ctx, l.group);
                    draw_thumb(&lp, thumb, media, tex, &ext, seed, &theme, 7.0, a);
                    let g = &d.groups[l.group];
                    // texts
                    let freed = g.freed();
                    let size_txt = if freed > 0 { fmt_size(freed) } else { "—".into() };
                    let sw = lp.layout_no_wrap(size_txt.clone(), bold(13.0), theme.text).size().x.max(64.0);
                    let warn_w = if g.none_kept() { 120.0 } else { 0.0 };
                    let tx = thumb.right() + 12.0;
                    let room = rr.right() - 16.0 - sw - 16.0 - warn_w - tx;
                    let name = widgets::truncate(&lp, &g.name, &bold(13.5), room);
                    lp.text(Pos2::new(tx, rr.center().y - 9.0), Align2::LEFT_CENTER, name, bold(13.5), with_alpha(theme.text, a));
                    let mut sub = trf("{n} copies", &[("n", &g.copies.len())]);
                    sub += &format!(" · {}", g.ext);
                    if let Some((w, h)) = g.meta.dims {
                        sub += &format!(" · {w}×{h}");
                    }
                    if let Some(s) = g.meta.duration {
                        sub += &format!(" · {}", fmt_duration(s));
                    }
                    let sub = widgets::truncate(&lp, &sub, &font(11.5), room);
                    lp.text(Pos2::new(tx, rr.center().y + 10.0), Align2::LEFT_CENTER, sub, font(11.5), with_alpha(theme.text_faint, a));
                    lp.text(Pos2::new(rr.right() - 16.0, rr.center().y - 9.0), Align2::RIGHT_CENTER, size_txt, bold(13.0), with_alpha(if freed > 0 { theme.text } else { theme.text_faint }, a));
                    lp.text(Pos2::new(rr.right() - 16.0, rr.center().y + 10.0), Align2::RIGHT_CENTER, trf("{size} each", &[("size", &fmt_size(g.size))]), font(11.5), with_alpha(theme.text_faint, a));
                    if g.none_kept() {
                        let wx = rr.right() - 16.0 - sw - 16.0;
                        lp.text(Pos2::new(wx, rr.center().y), Align2::RIGHT_CENTER, tr("no copy kept"), bold(11.5), theme.danger);
                        widgets::draw_icon(&lp, Icon::Warning, Rect::from_center_size(Pos2::new(wx - 92.0, rr.center().y), Vec2::splat(13.0)), theme.danger);
                    }
                    if cb_resp.clicked() {
                        click = Some((*l, 1));
                    } else if resp.clicked() {
                        click = Some((*l, 0));
                    }
                }
                Some(ci) => {
                    let c = &g.copies[ci];
                    let rr = Rect::from_min_size(Pos2::new(list.left() + 44.0, y), Vec2::new(list.width() - 10.0 - 50.0, COPY_H - 2.0));
                    let id = Id::new(("dupe-c", g.id, ci));
                    let resp = ui.interact(rr.intersect(list), id, Sense::click());
                    let sel = d.is_selected(l);
                    let h = ctx.animate_bool_with_time(id.with("h"), resp.hovered() || sel, 0.1);
                    if resp.hovered() {
                        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    lp.rect_filled(rr, cr(8.0), with_alpha(if sel { lerp_color(theme.surface_hi, theme.accent, 0.22) } else { theme.surface_hi }, h));
                    // tree guide
                    let gx = list.left() + 44.0 - 0.5;
                    lp.line_segment([Pos2::new(gx, y - 2.0), Pos2::new(gx, y + if ci + 1 == g.copies.len() { COPY_H / 2.0 } else { COPY_H })], Stroke::new(1.0, with_alpha(theme.stroke, 0.8)));
                    lp.line_segment([Pos2::new(gx, rr.center().y), Pos2::new(gx + 8.0, rr.center().y)], Stroke::new(1.0, with_alpha(theme.stroke, 0.8)));
                    let cb = Rect::from_center_size(Pos2::new(rr.left() + 22.0, rr.center().y), Vec2::splat(16.0));
                    let cb_resp = ui.interact(cb.expand(4.0).intersect(list), id.with("cb"), if c.locked { Sense::hover() } else { Sense::click() });
                    if c.locked {
                        // inside an app's library: cannot be removed from here
                        widgets::draw_icon(&lp, Icon::Shield, cb, theme.text_faint);
                        cb_resp.clone().on_hover_text(tr("Inside an app's library (Photos, Final Cut…): always kept"));
                    } else {
                        draw_checkbox(&lp, cb, if c.marked { 2 } else { 0 }, cb_resp.hovered(), &theme, 1.0);
                    }
                    // keep / remove tag
                    let mut x = rr.left() + 40.0;
                    if !c.marked {
                        x += tag(&lp, Pos2::new(x, rr.center().y), tr("KEEP"), theme.ok, &theme) + 8.0;
                    }
                    if c.protected {
                        widgets::draw_icon(&lp, Icon::Shield, Rect::from_center_size(Pos2::new(x + 6.0, rr.center().y), Vec2::splat(12.0)), theme.ok);
                        x += 20.0;
                    }
                    if c.shares && c.marked {
                        x += tag(&lp, Pos2::new(x, rr.center().y), tr("CLONE"), theme.text_dim, &theme) + 8.0;
                    }
                    let place_icon = place_icon(d.places.get(c.place).map(|p| p.kind).unwrap_or(PlaceKind::Folder));
                    widgets::draw_icon(&lp, place_icon, Rect::from_center_size(Pos2::new(x + 7.0, rr.center().y), Vec2::splat(13.0)), place_color(c.place, &theme));
                    x += 22.0;
                    let date = fmt_date(c.modified);
                    let dw = lp.layout_no_wrap(date.clone(), font(11.5), theme.text).size().x;
                    let shown = DupesUi::shown(c, &g.name);
                    let path = truncate_middle(&lp, &shown, &font(12.5), rr.right() - 16.0 - dw - 20.0 - x);
                    if resp.hovered() {
                        resp.clone().on_hover_text(&c.path);
                    }
                    let tc = if c.marked { theme.text_dim } else { theme.text };
                    lp.text(Pos2::new(x, rr.center().y), Align2::LEFT_CENTER, path, font(12.5), tc);
                    lp.text(Pos2::new(rr.right() - 14.0, rr.center().y), Align2::RIGHT_CENTER, date, font(11.5), theme.text_faint);
                    if cb_resp.clicked() {
                        click = Some((*l, 1));
                    } else if resp.clicked() {
                        click = Some((*l, 0));
                    }
                }
            }
        }
        if let Some((l, what)) = click {
            let gid = d.groups[l.group].id;
            match (l.copy, what) {
                (None, 1) => d.toggle_group(l.group),
                (Some(ci), 1) => d.toggle_copy(l.group, ci),
                (None, _) => {
                    let was = d.selected.map(|s| s.0) == Some(gid);
                    if !d.expanded.insert(gid) && was {
                        d.expanded.remove(&gid);
                    }
                    d.select_line(&l);
                }
                (Some(_), _) => d.select_line(&l),
            }
            self.sfx(Sfx::Blip);
        }

        // scrollbar
        if max_scroll > 0.0 {
            let track = Rect::from_min_max(Pos2::new(list.right() - 5.0, list.top() + 2.0), Pos2::new(list.right(), list.bottom() - 2.0));
            let thumb_h = (track.height() * list.height() / d.content_h).max(28.0);
            let t = d.scroll / max_scroll;
            let thumb = Rect::from_min_size(Pos2::new(track.left(), track.top() + (track.height() - thumb_h) * t), Vec2::new(track.width(), thumb_h));
            let resp = ui.interact(thumb.expand2(Vec2::new(4.0, 0.0)), Id::new("dupes-scroll"), Sense::drag());
            if resp.dragged() {
                d.scroll = (d.scroll + resp.drag_delta().y * max_scroll / (track.height() - thumb_h).max(1.0)).clamp(0.0, max_scroll);
            }
            let h = ctx.animate_bool_with_time(Id::new("dupes-scroll-h"), resp.hovered() || resp.dragged(), 0.12);
            let p = ui.painter();
            p.rect_filled(thumb, cr(3.0), lerp_color(with_alpha(theme.text_faint, 0.6), theme.text_dim, h));
        }
        // fade the edges so rows do not stop on a hard line
        let p = ui.painter();
        let bg = lerp_color(theme.bg_top, theme.bg_bottom, ((list.bottom() - p.clip_rect().top()) / p.clip_rect().height().max(1.0)).clamp(0.0, 1.0));
        if d.scroll > 0.0 {
            let bg_top = lerp_color(theme.bg_top, theme.bg_bottom, ((list.top() - p.clip_rect().top()) / p.clip_rect().height().max(1.0)).clamp(0.0, 1.0));
            widgets::vgradient(p, Rect::from_min_size(list.left_top(), Vec2::new(list.width() - 8.0, 14.0)), bg_top, with_alpha(bg_top, 0.0));
        }
        if d.scroll < max_scroll {
            widgets::vgradient(p, Rect::from_min_max(Pos2::new(list.left(), list.bottom() - 18.0), Pos2::new(list.right() - 8.0, list.bottom())), with_alpha(bg, 0.0), bg);
        }
    }

    /// Bottom bar: what is selected and the button that removes it.
    fn dupes_actions(&mut self, ui: &mut Ui, r: Rect, d: &mut DupesUi) {
        let theme = self.theme.clone();
        card(ui.painter(), r, &theme);
        let v = d.view();
        let (n, bytes, unsafe_groups) = (v.marked, v.freed, v.unsafe_groups);
        let p = ui.painter();
        let x = r.left() + 20.0;
        if n == 0 {
            p.text(Pos2::new(x, r.center().y), Align2::LEFT_CENTER, tr("Check the copies you want to remove"), font(13.0), theme.text_dim);
        } else {
            p.text(Pos2::new(x, r.center().y - 9.0), Align2::LEFT_CENTER, trf("{n} copies selected", &[("n", &fmt_count(n as u64))]), bold(13.5), theme.text);
            let sub = if unsafe_groups > 0 {
                (trf("{n} files would lose every copy", &[("n", &unsafe_groups)]), theme.danger)
            } else {
                (trf("{size} will be freed", &[("size", &fmt_size(bytes))]), theme.text_dim)
            };
            p.text(Pos2::new(x, r.center().y + 10.0), Align2::LEFT_CENTER, sub.0, font(12.0), sub.1);
        }
        let trash = Rect::from_min_size(Pos2::new(r.right() - 14.0 - 206.0, r.center().y - 17.0), Vec2::new(206.0, 34.0));
        let clear = Rect::from_min_size(Pos2::new(trash.left() - 10.0 - 84.0, trash.top()), Vec2::new(84.0, 34.0));
        let auto = Rect::from_min_size(Pos2::new(clear.left() - 8.0 - 136.0, trash.top()), Vec2::new(136.0, 34.0));
        if r.width() > 620.0 {
            if widgets::button(ui, auto, Id::new("dupes-auto"), tr("Auto-select"), Some(Icon::Bolt), BtnStyle::Ghost, &theme).clicked() {
                for g in d.groups.iter_mut() {
                    g.manual = false;
                }
                d.reapply();
                self.sfx(Sfx::Blip);
            }
            if widgets::button(ui, clear, Id::new("dupes-clear"), tr("Clear"), None, BtnStyle::Ghost, &theme).clicked() {
                for g in d.groups.iter_mut() {
                    g.manual = true;
                    g.copies.iter_mut().for_each(|c| c.marked = false);
                }
                d.touch();
                self.sfx(Sfx::BlipDown);
            }
        }
        let ready = n > 0 && unsafe_groups == 0 && !d.is_scanning() && d.removing.is_none();
        // split button: the Trash, and a menu with "Delete permanently"
        let main = Rect::from_min_max(trash.min, Pos2::new(trash.right() - 31.0, trash.bottom()));
        let more = Rect::from_min_max(Pos2::new(trash.right() - 30.0, trash.top()), trash.max);
        let left_r = egui::CornerRadius { nw: 7, sw: 7, ne: 0, se: 0 };
        let right_r = egui::CornerRadius { nw: 0, sw: 0, ne: 7, se: 7 };
        let resp = widgets::button_shaped(ui, main, Id::new("dupes-trash"), tr("Move to Trash"), Some(Icon::Trash), BtnStyle::Danger, &theme, ready, left_r);
        let resp = if n > 0 && d.is_scanning() { resp.on_hover_text(tr("Available once the comparison is over")) } else { resp };
        let more_resp = widgets::button_shaped(ui, more, Id::new("dupes-trash-more"), "", Some(Icon::Expand), BtnStyle::Danger, &theme, ready, right_r).on_hover_text(tr("More ways to remove"));
        let mut open = None;
        if resp.clicked() {
            open = Some(false);
        }
        if more_resp.clicked() {
            d.trash_menu = !d.trash_menu;
        }
        if d.trash_menu {
            // opens upwards: the bar is at the bottom of the window
            let items = [(Icon::Trash, tr("Move to Trash…"), false), (Icon::Warning, tr("Delete permanently…"), true)];
            let mw = 230.0;
            let mh = items.len() as f32 * 34.0 + 12.0;
            let origin = Pos2::new(trash.right() - mw, trash.top() - 8.0 - mh);
            let mut menu_rect = Rect::NOTHING;
            egui::Area::new(Id::new("dupes-trash-menu")).order(egui::Order::Foreground).fixed_pos(origin).show(ui.ctx(), |ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(mw, mh), Sense::click());
                menu_rect = rect;
                let p = ui.painter();
                widgets::shadow(p, rect, 12.0, 1.0, &theme);
                p.rect_filled(rect, cr(12.0), theme.surface_hi);
                p.rect_stroke(rect, cr(12.0), Stroke::new(1.0, theme.stroke), egui::StrokeKind::Inside);
                for (k, (icon, label, permanent)) in items.iter().enumerate() {
                    let rr = Rect::from_min_size(rect.left_top() + Vec2::new(6.0, 6.0 + k as f32 * 34.0), Vec2::new(mw - 12.0, 32.0));
                    let id = Id::new(("dupes-trash-item", k));
                    let r = ui.interact(rr, id, Sense::click());
                    let hh = ui.ctx().animate_bool_with_time(id, r.hovered(), 0.08);
                    let col = if *permanent { theme.danger } else { theme.accent };
                    let p = ui.painter();
                    p.rect_filled(rr, cr(8.0), with_alpha(col, 0.85 * hh));
                    widgets::draw_icon(p, *icon, Rect::from_center_size(Pos2::new(rr.left() + 18.0, rr.center().y), Vec2::splat(14.0)), lerp_color(col, Color32::WHITE, hh));
                    p.text(Pos2::new(rr.left() + 36.0, rr.center().y), Align2::LEFT_CENTER, *label, bold(13.0), lerp_color(theme.text, Color32::WHITE, hh));
                    if r.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if r.clicked() {
                        open = Some(*permanent);
                    }
                }
            });
            let clicked_out = ui.input(|i| i.pointer.any_pressed()) && ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| !menu_rect.contains(p) && !more.contains(p));
            if clicked_out || open.is_some() {
                d.trash_menu = false;
            }
        }
        if let Some(permanent) = open {
            if d.is_mock() {
                self.toasts.push(trf("Preview only: {n} copies ({size}) would go to the Trash", &[("n", &fmt_count(n as u64)), ("size", &fmt_size(bytes))]), theme.accent);
            } else {
                self.open_modal(Modal::DupesConfirm { ack: false, permanent });
            }
        }
    }
}

fn dupes_toolbar(ui: &mut Ui, r: Rect, d: &mut DupesUi, theme: &Theme) {
    // search
    let sw = (r.width() * 0.36).clamp(180.0, 300.0);
    let field = Rect::from_min_size(r.min, Vec2::new(sw, r.height()));
    let id = Id::new("dupes-search");
    let focused = ui.memory(|m| m.has_focus(id));
    let t = ui.ctx().animate_bool_with_time(id.with("f"), focused, 0.15);
    {
        let p = ui.painter();
        p.rect_filled(field, cr(8.0), lerp_color(with_alpha(theme.bg_bottom, 0.55), with_alpha(theme.bg_bottom, 0.9), t));
        p.rect_stroke(field, cr(8.0), Stroke::new(1.0, lerp_color(with_alpha(theme.stroke, 0.9), theme.accent, t)), egui::StrokeKind::Inside);
        widgets::draw_icon(p, Icon::Search, Rect::from_center_size(Pos2::new(field.left() + 17.0, field.center().y), Vec2::splat(14.0)), lerp_color(theme.text_faint, theme.accent, t));
    }
    let inner = Rect::from_min_max(Pos2::new(field.left() + 32.0, field.top()), Pos2::new(field.right() - if d.search.is_empty() { 10.0 } else { 30.0 }, field.bottom()));
    let edit = egui::TextEdit::singleline(&mut d.search)
        .id(id)
        .frame(egui::Frame::NONE)
        .font(font(13.0))
        .text_color(theme.text)
        .hint_text(egui::RichText::new(tr("Search names and folders")).color(theme.text_faint).font(font(13.0)))
        .desired_width(inner.width())
        .vertical_align(egui::Align::Center);
    if ui.put(inner, edit).changed() {
        d.scroll = 0.0;
    }
    if !d.search.is_empty() {
        let x = Rect::from_center_size(Pos2::new(field.right() - 16.0, field.center().y), Vec2::splat(20.0));
        if widgets::icon_button(ui, x, id.with("x"), Icon::Close, theme, true).clicked() {
            d.search.clear();
        }
    }

    // filter
    let segs = [(Filter::All, tr("All")), (Filter::Photos, tr("Photos")), (Filter::Videos, tr("Videos")), (Filter::Other, tr("Other"))];
    let n = d.view().counts;
    let counts = |f: Filter| n[segs.iter().position(|s| s.0 == f).unwrap_or(0)];
    let p = ui.painter().clone();
    let widths: Vec<f32> = segs
        .iter()
        .map(|(f, l)| p.layout_no_wrap(l.to_string(), bold(12.5), Color32::WHITE).size().x + p.layout_no_wrap(counts(*f).to_string(), font(11.5), Color32::WHITE).size().x + 30.0)
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 6.0;
    let seg_rect = Rect::from_min_size(Pos2::new(field.right() + 12.0, r.top()), Vec2::new(total, r.height()));
    p.rect_filled(seg_rect, cr(8.0), with_alpha(theme.bg_bottom, 0.55));
    p.rect_stroke(seg_rect, cr(8.0), Stroke::new(1.0, with_alpha(theme.stroke, 0.9)), egui::StrokeKind::Inside);
    let mut x = seg_rect.left() + 3.0;
    for (k, (f, label)) in segs.iter().enumerate() {
        let rr = Rect::from_min_size(Pos2::new(x, seg_rect.top() + 3.0), Vec2::new(widths[k], seg_rect.height() - 6.0));
        let sid = Id::new(("dupes-seg", k));
        let resp = ui.interact(rr, sid, Sense::click());
        let on = ui.ctx().animate_bool_with_time(sid.with("on"), d.filter == *f, 0.15);
        let h = ui.ctx().animate_bool_with_time(sid.with("h"), resp.hovered(), 0.1);
        p.rect_filled(rr, cr(6.0), lerp_color(with_alpha(theme.surface_hi, 0.6 * h), theme.surface_hi, on));
        let lc = lerp_color(theme.text_dim, theme.text, on.max(h));
        let lw = p.layout_no_wrap(label.to_string(), bold(12.5), lc).size().x;
        p.text(Pos2::new(rr.left() + 10.0, rr.center().y), Align2::LEFT_CENTER, *label, bold(12.5), lc);
        p.text(Pos2::new(rr.left() + 16.0 + lw, rr.center().y), Align2::LEFT_CENTER, counts(*f).to_string(), font(11.5), lerp_color(theme.text_faint, theme.accent, on));
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            d.filter = *f;
            d.scroll = 0.0;
        }
        x += widths[k];
    }

    // sort
    let label = match d.sort {
        Sort::Size => tr("Biggest first"),
        Sort::Copies => tr("Most copies first"),
        Sort::Name => tr("By name"),
    };
    let lw = p.layout_no_wrap(label.to_string(), bold(12.5), Color32::WHITE).size().x;
    let sr = Rect::from_min_size(Pos2::new(r.right() - lw - 40.0, r.top()), Vec2::new(lw + 40.0, r.height()));
    if sr.left() > seg_rect.right() + 8.0 {
        let sid = Id::new("dupes-sort");
        let resp = ui.interact(sr, sid, Sense::click());
        let h = ui.ctx().animate_bool_with_time(sid, resp.hovered(), 0.1);
        p.rect_filled(sr, cr(8.0), with_alpha(theme.surface_hi, 0.8 * h));
        p.text(Pos2::new(sr.left() + 12.0, sr.center().y), Align2::LEFT_CENTER, label, bold(12.5), lerp_color(theme.text_dim, theme.text, h));
        widgets::draw_icon(&p, Icon::Expand, Rect::from_center_size(Pos2::new(sr.right() - 16.0, sr.center().y), Vec2::splat(10.0)), theme.text_faint);
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            d.sort = match d.sort {
                Sort::Size => Sort::Copies,
                Sort::Copies => Sort::Name,
                Sort::Name => Sort::Size,
            };
        }
    }
}

/// The "always keep" rules, as chips that wrap. Returns the height used.
fn dupes_rules(ui: &mut Ui, r: Rect, d: &mut DupesUi, theme: &Theme) -> f32 {
    let p = ui.painter().clone();
    let chip_h = 28.0;
    let gap = 8.0;
    widgets::draw_icon(&p, Icon::Shield, Rect::from_center_size(Pos2::new(r.left() + 8.0, r.top() + chip_h / 2.0), Vec2::splat(14.0)), theme.ok);
    p.text(Pos2::new(r.left() + 22.0, r.top() + chip_h / 2.0), Align2::LEFT_CENTER, tr("Always keep"), bold(12.5), theme.text_dim);
    let prefer_label = match d.prefer {
        Prefer::Oldest => tr("the oldest copy"),
        Prefer::Newest => tr("the newest copy"),
        Prefer::ShortestPath => tr("the shortest path"),
    };
    let otherwise = tr("Otherwise keep");
    let ow = p.layout_no_wrap(otherwise.to_string(), font(12.0), Color32::WHITE).size().x;
    let pw = p.layout_no_wrap(prefer_label.to_string(), bold(12.5), Color32::WHITE).size().x + 34.0;
    let pref = Rect::from_min_size(Pos2::new(r.right() - pw, r.top()), Vec2::new(pw, chip_h));
    let right_limit = pref.left() - ow - 20.0;

    let start_x = r.left() + 22.0 + p.layout_no_wrap(tr("Always keep").to_string(), bold(12.5), Color32::WHITE).size().x + 12.0;
    let (mut x, mut y) = (start_x, r.top());
    let mut toggle = None;
    let mut remove = None;
    let place = |x: &mut f32, y: &mut f32, w: f32| -> Rect {
        let limit = if *y == r.top() { right_limit } else { r.right() };
        if *x + w > limit && *x > start_x {
            *x = start_x;
            *y += chip_h + 6.0;
        }
        let rr = Rect::from_min_size(Pos2::new(*x, *y), Vec2::new(w, chip_h));
        *x += w + gap;
        rr
    };
    for (k, rule) in d.rules.iter().enumerate() {
        let txt = widgets::truncate(&p, &rule.pattern, &font(12.5), 240.0);
        let tw = p.layout_no_wrap(txt.clone(), font(12.5), Color32::WHITE).size().x;
        let rr = place(&mut x, &mut y, tw + 56.0);
        let id = Id::new(("dupes-rule", k));
        let resp = ui.interact(rr, id, Sense::click());
        let h = ui.ctx().animate_bool_with_time(id.with("h"), resp.hovered(), 0.1);
        let on = ui.ctx().animate_bool_with_time(id.with("on"), rule.on, 0.15);
        p.rect_filled(rr, cr(14.0), lerp_color(with_alpha(theme.bg_bottom, 0.5), with_alpha(theme.ok, 0.16), on));
        p.rect_stroke(rr, cr(14.0), Stroke::new(1.0, lerp_color(with_alpha(theme.stroke, 0.9), with_alpha(theme.ok, 0.6), on)), egui::StrokeKind::Inside);
        if h > 0.0 {
            p.rect_filled(rr, cr(14.0), with_alpha(theme.surface_hi, 0.35 * h));
        }
        let dot = Pos2::new(rr.left() + 15.0, rr.center().y);
        p.circle_stroke(dot, 5.0, Stroke::new(1.5, lerp_color(theme.text_faint, theme.ok, on)));
        p.circle_filled(dot, 3.0 * on, theme.ok);
        p.text(Pos2::new(rr.left() + 28.0, rr.center().y), Align2::LEFT_CENTER, txt, font(12.5), lerp_color(theme.text_faint, theme.text, on));
        let xr = Rect::from_center_size(Pos2::new(rr.right() - 14.0, rr.center().y), Vec2::splat(18.0));
        let x_resp = ui.interact(xr, id.with("x"), Sense::click());
        let xh = ui.ctx().animate_bool_with_time(id.with("xh"), x_resp.hovered(), 0.1);
        p.circle_filled(xr.center(), 9.0, with_alpha(theme.surface_hi, xh));
        widgets::draw_icon(&p, Icon::Close, Rect::from_center_size(xr.center(), Vec2::splat(7.0)), lerp_color(theme.text_faint, theme.text, xh.max(h * 0.6)));
        if resp.hovered() || x_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if x_resp.clicked() {
            remove = Some(k);
        } else if resp.clicked() {
            toggle = Some(k);
        }
    }
    // new rule
    if let Some(mut draft) = d.rule_draft.take() {
        let rr = place(&mut x, &mut y, 300.0);
        let id = Id::new("dupes-rule-draft");
        let resp = widgets::text_input(ui, rr, id, &mut draft, tr("**/Archive/**  or  ~/Pictures"), theme);
        if d.draft_focus {
            resp.request_focus();
            d.draft_focus = false;
        }
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        if resp.lost_focus() {
            if enter && !draft.trim().is_empty() {
                d.rules.push(Rule { pattern: draft.trim().to_string(), on: true });
                d.reapply();
            }
        } else {
            d.rule_draft = Some(draft);
        }
    } else {
        let label = tr("Add rule");
        let tw = p.layout_no_wrap(label.to_string(), bold(12.5), Color32::WHITE).size().x;
        let rr = place(&mut x, &mut y, tw + 40.0);
        let id = Id::new("dupes-rule-add");
        let resp = ui.interact(rr, id, Sense::click());
        let h = ui.ctx().animate_bool_with_time(id, resp.hovered(), 0.1);
        p.rect_stroke(rr, cr(14.0), Stroke::new(1.0, lerp_color(with_alpha(theme.stroke, 0.9), theme.accent, h)), egui::StrokeKind::Inside);
        widgets::draw_icon(&p, Icon::Plus, Rect::from_center_size(Pos2::new(rr.left() + 16.0, rr.center().y), Vec2::splat(10.0)), lerp_color(theme.text_dim, theme.accent, h));
        p.text(Pos2::new(rr.left() + 28.0, rr.center().y), Align2::LEFT_CENTER, label, bold(12.5), lerp_color(theme.text_dim, theme.text, h));
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            d.rule_draft = Some(String::new());
            d.draft_focus = true;
        }
    }
    if let Some(k) = toggle {
        d.rules[k].on = !d.rules[k].on;
        d.reapply();
    }
    if let Some(k) = remove {
        d.rules.remove(k);
        d.reapply();
    }

    // otherwise: which copy wins
    p.text(Pos2::new(pref.left() - 8.0, pref.center().y), Align2::RIGHT_CENTER, otherwise, font(12.0), theme.text_faint);
    let id = Id::new("dupes-prefer");
    let resp = ui.interact(pref, id, Sense::click());
    let h = ui.ctx().animate_bool_with_time(id, resp.hovered(), 0.1);
    p.rect_filled(pref, cr(8.0), lerp_color(with_alpha(theme.bg_bottom, 0.5), theme.surface_hi, h));
    p.rect_stroke(pref, cr(8.0), Stroke::new(1.0, with_alpha(theme.stroke, 0.9)), egui::StrokeKind::Inside);
    p.text(Pos2::new(pref.left() + 12.0, pref.center().y), Align2::LEFT_CENTER, prefer_label, bold(12.5), theme.text);
    widgets::draw_icon(&p, Icon::Expand, Rect::from_center_size(Pos2::new(pref.right() - 14.0, pref.center().y), Vec2::splat(9.0)), theme.text_faint);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if resp.clicked() {
        d.prefer = d.prefer.next();
        d.reapply();
    }
    y + chip_h - r.top()
}

/// Right panel: a preview of the selected file, what it is and its copies.
fn dupes_preview(ui: &mut Ui, panel: Rect, d: &mut DupesUi, theme: &Theme) {
    let p = ui.painter().clone();
    widgets::shadow(&p, panel, 18.0, 0.5, theme);
    p.rect_filled(panel, cr(12.0), with_alpha(theme.surface, if theme.dark { 0.72 } else { 0.9 }));
    p.rect_stroke(panel, cr(12.0), Stroke::new(1.0, with_alpha(theme.stroke, 0.6)), egui::StrokeKind::Inside);
    let inner = panel.shrink2(Vec2::new(18.0, 18.0));
    let Some((gid, ci)) = d.selected else {
        let c = panel.center() - Vec2::new(0.0, 30.0);
        widgets::glow(&p, c, 70.0, with_alpha(theme.accent, 0.12));
        widgets::draw_icon(&p, Icon::Copies, Rect::from_center_size(c, Vec2::splat(44.0)), with_alpha(theme.accent, 0.8));
        p.text(c + Vec2::new(0.0, 52.0), Align2::CENTER_CENTER, tr("Pick a file to see it here"), bold(14.0), theme.text);
        let g = p.layout(tr("Photos, RAW files and videos show a preview. Every copy is listed with where it lives.").to_string(), font(12.0), theme.text_dim, inner.width() - 40.0);
        let gw = g.size().x;
        p.galley(Pos2::new(c.x - gw / 2.0, c.y + 66.0), g, theme.text_dim);
        return;
    };
    let Some(gi) = d.group_by_id(gid) else {
        d.selected = None;
        return;
    };
    let ctx = ui.ctx().clone();
    let (media, seed, ext) = (d.groups[gi].media, d.groups[gi].seed, d.groups[gi].ext.clone());
    let tex = d.thumb(&ctx, gi);
    d.ask_meta(gi);
    let meta = d.meta_of(gi);
    let is_local = |d: &DupesUi, place: usize| matches!(d.places.get(place).and_then(|p| p.spec.as_ref()), Some(Spec::Local(_)));
    let g = &d.groups[gi];
    let ci = ci.min(g.copies.len().saturating_sub(1));
    let sel_path = g.copies.get(ci).map(|c| c.path.clone()).unwrap_or_default();
    let sel_local = g.copies.get(ci).is_some_and(|c| is_local(d, c.place)) && !d.is_mock();

    // preview
    let ph = (inner.width() * 0.68).min(inner.height() * 0.42);
    let pr = Rect::from_min_size(inner.min, Vec2::new(inner.width(), ph));
    p.rect_filled(pr, cr(10.0), with_alpha(theme.bg_bottom, 0.8));
    match tex {
        Some((tex, img_aspect)) => {
            // the picture's own shape, letterboxed in the frame
            let aspect = img_aspect.clamp(0.3, 4.0);
            let fr = if pr.width() / pr.height() > aspect {
                Rect::from_center_size(pr.center(), Vec2::new(pr.height() * aspect, pr.height()))
            } else {
                Rect::from_center_size(pr.center(), Vec2::new(pr.width(), pr.width() / aspect))
            };
            p.add(RectShape::filled(fr, cr(10.0), Color32::WHITE).with_texture(tex, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))));
            if media == Media::Video {
                let c = fr.center();
                p.circle_filled(c, 26.0, with_alpha(Color32::BLACK, 0.45));
                p.circle_stroke(c, 26.0, Stroke::new(1.5, with_alpha(Color32::WHITE, 0.8)));
                widgets::draw_icon(&p, Icon::Play, Rect::from_center_size(c + Vec2::new(2.0, 0.0), Vec2::splat(20.0)), Color32::WHITE);
                if let Some(s) = meta.duration {
                    badge(&p, fr.right_bottom() - Vec2::new(10.0, 10.0), Align2::RIGHT_BOTTOM, &fmt_duration(s));
                }
            }
            if meta.dims.is_some_and(|(w, _)| w >= 3840) && media == Media::Video {
                badge(&p, fr.left_top() + Vec2::new(10.0, 10.0), Align2::LEFT_TOP, "4K");
            }
            if media == Media::Raw {
                badge(&p, fr.left_top() + Vec2::new(10.0, 10.0), Align2::LEFT_TOP, &format!("RAW · {}", g.ext));
            }
        }
        None => draw_file_art(&p, pr, media, &ext, seed, theme),
    }
    // open it, or show it in the Finder
    if sel_local {
        let open_r = Rect::from_center_size(Pos2::new(pr.right() - 22.0, pr.top() + 22.0), Vec2::splat(30.0));
        let reveal_r = open_r.translate(Vec2::new(-36.0, 0.0));
        for (r, icon, tip, reveal) in [(open_r, Icon::LinkOut, tr("Open"), false), (reveal_r, Icon::Folder, if cfg!(target_os = "macos") { tr("Reveal in Finder") } else { tr("Show in file manager") }, true)] {
            let id = Id::new(("dupes-open", reveal));
            let resp = ui.interact(r, id, Sense::click()).on_hover_text(tip);
            let h = ui.ctx().animate_bool_with_time(id, resp.hovered(), 0.1);
            p.circle_filled(r.center(), 15.0, with_alpha(Color32::BLACK, 0.4 + 0.25 * h));
            widgets::draw_icon(&p, icon, Rect::from_center_size(r.center(), Vec2::splat(13.0)), Color32::WHITE);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                super::app::os_open(&sel_path, reveal);
            }
        }
    }

    // title
    let mut y = pr.bottom() + 16.0;
    let size = fmt_size(g.size);
    let sw = p.layout_no_wrap(size.clone(), display(18.0), theme.text).size().x;
    let name = widgets::truncate(&p, &g.name, &display(18.0), inner.width() - sw - 14.0);
    p.text(Pos2::new(inner.left(), y), Align2::LEFT_TOP, name, display(18.0), theme.text);
    p.text(Pos2::new(inner.right(), y), Align2::RIGHT_TOP, size, display(18.0), theme.text);
    y += 28.0;
    let kind = match media {
        Media::Photo => tr("Photo"),
        Media::Raw => tr("RAW photo"),
        Media::Video => tr("Video"),
        Media::Audio => tr("Audio"),
        Media::Other => tr("File"),
    };
    p.text(Pos2::new(inner.left(), y), Align2::LEFT_TOP, trf("{kind} · {n} identical copies", &[("kind", &kind), ("n", &g.copies.len())]), font(12.0), theme.text_faint);
    y += 26.0;

    // facts
    let mut facts: Vec<(&str, String)> = Vec::new();
    if let Some((w, h)) = meta.dims {
        facts.push((tr("Dimensions"), format!("{w} × {h}")));
    }
    if let Some(s) = meta.duration {
        facts.push((tr("Duration"), fmt_duration(s)));
    }
    if let Some(t) = meta.taken {
        facts.push((if media.is_visual() { tr("Taken") } else { tr("Created") }, fmt_date(t)));
    }
    if let Some(dev) = &meta.device {
        facts.push((tr("Device"), dev.clone()));
    }
    let hx = &g.digest;
    if hx.len() > 16 {
        facts.push((tr("Fingerprint"), format!("{} … {}", &hx[..8], &hx[hx.len() - 8..])));
    }
    // as wide as the longest label (they vary a lot between languages)
    let label_w = facts.iter().map(|(k, _)| p.layout_no_wrap(k.to_string(), font(12.0), Color32::WHITE).size().x).fold(0.0f32, f32::max).max(82.0) + 14.0;
    let label_w = label_w.min(inner.width() * 0.45);
    for (k, v) in facts.iter() {
        p.text(Pos2::new(inner.left(), y), Align2::LEFT_TOP, *k, font(12.0), theme.text_faint);
        let v = widgets::truncate(&p, v, &font(12.5), inner.width() - label_w);
        p.text(Pos2::new(inner.left() + label_w, y), Align2::LEFT_TOP, v, font(12.5), theme.text);
        y += 21.0;
    }
    y += 10.0;
    p.line_segment([Pos2::new(inner.left(), y), Pos2::new(inner.right(), y)], Stroke::new(1.0, with_alpha(theme.stroke, 0.6)));
    y += 14.0;
    super::home::section_label(ui, Pos2::new(inner.left(), y), tr("COPIES"), theme);
    y += 22.0;

    // copies, each with its whole path (wrapped)
    let mut toggle = None;
    let mut pick = None;
    for (k, c) in g.copies.iter().enumerate() {
        let label = if c.locked { tr("Library") } else if c.marked { tr("Remove") } else { tr("Keep") };
        let col = if c.locked { theme.text_dim } else if c.marked { theme.danger } else { theme.ok };
        let lw = p.layout_no_wrap(label.to_string(), bold(11.5), col).size().x;
        let left_x = inner.left() - 8.0;
        let tx = left_x + 46.0;
        let room = inner.right() + 8.0 - 10.0 - (lw + 22.0) - 10.0 - tx;
        let mut path_g = p.layout(c.path.clone(), font(11.5), theme.text_dim, room);
        if path_g.rows.len() > 3 {
            // very deep paths: keep the start and the end, on three lines at most
            let short = truncate_middle(&p, &c.path, &font(11.5), room * 2.7);
            path_g = p.layout(short, font(11.5), theme.text_dim, room);
        }
        let row_h = 30.0 + path_g.size().y + 10.0;
        let rr = Rect::from_min_size(Pos2::new(left_x, y), Vec2::new(inner.width() + 16.0, row_h - 4.0));
        if rr.bottom() > inner.bottom() {
            let more = g.copies.len() - k;
            p.text(Pos2::new(inner.left(), y + 8.0), Align2::LEFT_TOP, trf("+{n} more", &[("n", &more)]), font(12.0), theme.text_dim);
            break;
        }
        let id = Id::new(("dupes-pc", gid, k));
        let resp = ui.interact(rr, id, Sense::click());
        let sel = k == ci;
        let h = ui.ctx().animate_bool_with_time(id.with("h"), resp.hovered() || sel, 0.1);
        p.rect_filled(rr, cr(8.0), with_alpha(if sel { lerp_color(theme.surface_hi, theme.accent, 0.2) } else { theme.surface_hi }, h));
        let place = d.places.get(c.place);
        let icon = place_icon(place.map(|p| p.kind).unwrap_or(PlaceKind::Folder));
        let tile = Rect::from_center_size(Pos2::new(rr.left() + 22.0, rr.top() + 22.0), Vec2::splat(28.0));
        p.rect_filled(tile, cr(7.0), with_alpha(place_color(c.place, theme), 0.18));
        widgets::draw_icon(&p, icon, tile.shrink(7.0), place_color(c.place, theme));
        // keep / remove switch (copies in a library always stay)
        let pill = Rect::from_min_size(Pos2::new(rr.right() - 10.0 - lw - 22.0, rr.top() + 11.0), Vec2::new(lw + 22.0, 22.0));
        let pill_resp = ui.interact(pill, id.with("pill"), if c.locked { Sense::hover() } else { Sense::click() });
        let ph = ui.ctx().animate_bool_with_time(id.with("ph"), pill_resp.hovered() && !c.locked, 0.1);
        p.rect_filled(pill, cr(11.0), with_alpha(col, 0.16 + 0.14 * ph));
        p.rect_stroke(pill, cr(11.0), Stroke::new(1.0, with_alpha(col, 0.5 + 0.4 * ph)), egui::StrokeKind::Inside);
        p.text(pill.center(), Align2::CENTER_CENTER, label, bold(11.5), col);
        if c.locked {
            pill_resp.clone().on_hover_text(tr("Inside an app's library (Photos, Final Cut…): always kept"));
        } else if pill_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let pl = place.map(|p| p.label.clone()).unwrap_or_default();
        let note = if c.protected {
            tr("protected by a rule").to_string()
        } else if c.shares && c.marked {
            tr("clone: frees no space").to_string()
        } else {
            fmt_date(c.modified)
        };
        let head = format!("{pl}  ·  {note}");
        p.text(Pos2::new(tx, rr.top() + 14.0), Align2::LEFT_CENTER, widgets::truncate(&p, &head, &bold(12.0), room), bold(12.0), theme.text);
        p.galley(Pos2::new(tx, rr.top() + 26.0), path_g, theme.text_dim);
        if pill_resp.clicked() {
            toggle = Some(k);
        } else if resp.double_clicked() && is_local(d, c.place) && !d.is_mock() {
            super::app::os_open(&c.path, true);
        } else if resp.clicked() {
            pick = Some(k);
        }
        if resp.hovered() && !pill_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        y += row_h;
    }
    if let Some(k) = toggle {
        d.toggle_copy(gi, k);
    }
    if let Some(k) = pick {
        d.selected = Some((gid, k));
        d.expanded.insert(gid);
        d.reveal = true;
    }
}

// ---------------------------------------------------------------------------
// drawing helpers

fn card(p: &Painter, r: Rect, theme: &Theme) {
    widgets::shadow(p, r, 12.0, 0.4, theme);
    p.rect_filled(r, cr(12.0), with_alpha(theme.surface, if theme.dark { 0.72 } else { 0.9 }));
    p.rect_stroke(r, cr(12.0), Stroke::new(1.0, with_alpha(theme.stroke, 0.6)), egui::StrokeKind::Inside);
}

/// 0 empty, 1 some, 2 checked.
fn draw_checkbox(p: &Painter, b: Rect, state: u8, hovered: bool, theme: &Theme, alpha: f32) {
    let fill = match state {
        0 => if hovered { theme.surface_hi } else { with_alpha(theme.bg_bottom, 0.6) },
        _ => if hovered { lighten(theme.accent, 0.06) } else { theme.accent },
    };
    p.rect_filled(b, cr(5.0), with_alpha(fill, alpha));
    if state == 0 {
        p.rect_stroke(b, cr(5.0), Stroke::new(1.2, with_alpha(if hovered { theme.accent } else { theme.text_faint }, alpha)), egui::StrokeKind::Inside);
    }
    match state {
        1 => {
            p.line_segment([b.center() - Vec2::new(b.width() * 0.25, 0.0), b.center() + Vec2::new(b.width() * 0.25, 0.0)], Stroke::new(2.0, with_alpha(Color32::WHITE, alpha)));
        }
        2 => widgets::draw_icon(p, Icon::Check, b.shrink(b.width() * 0.2), with_alpha(Color32::WHITE, alpha)),
        _ => {}
    }
}

/// Small rounded tag; returns its width.
fn tag(p: &Painter, left_center: Pos2, text: &str, color: Color32, _theme: &Theme) -> f32 {
    let g = p.layout_no_wrap(text.to_string(), bold(10.0), color);
    let r = Rect::from_min_size(Pos2::new(left_center.x, left_center.y - 9.0), Vec2::new(g.size().x + 14.0, 18.0));
    p.rect_filled(r, cr(9.0), with_alpha(color, 0.16));
    p.galley(Pos2::new(r.left() + 7.0, r.center().y - g.size().y / 2.0), g, color);
    r.width()
}

/// Dark label over an image (duration, 4K, RAW).
fn badge(p: &Painter, at: Pos2, align: Align2, text: &str) {
    let g = p.layout_no_wrap(text.to_string(), bold(11.0), Color32::WHITE);
    let size = g.size() + Vec2::new(12.0, 6.0);
    let r = align.anchor_size(at, size);
    p.rect_filled(r, cr(5.0), with_alpha(Color32::BLACK, 0.55));
    p.galley(r.min + Vec2::new(6.0, 3.0), g, Color32::WHITE);
}

#[allow(clippy::too_many_arguments)]
fn draw_thumb(p: &Painter, r: Rect, media: Media, tex: Option<(egui::TextureId, f32)>, ext: &str, seed: u32, theme: &Theme, radius: f32, alpha: f32) {
    match tex {
        Some((tex, aspect)) => {
            p.add(RectShape::filled(r, cr(radius), with_alpha(Color32::WHITE, alpha)).with_texture(tex, cover_uv(aspect, 1.0)));
            if media == Media::Video {
                p.circle_filled(r.center(), 8.0, with_alpha(Color32::BLACK, 0.45 * alpha));
                widgets::draw_icon(p, Icon::Play, Rect::from_center_size(r.center() + Vec2::new(0.8, 0.0), Vec2::splat(8.0)), with_alpha(Color32::WHITE, alpha));
            }
        }
        None => {
            let col = file_color(media, seed, theme);
            p.rect_filled(r, cr(radius), with_alpha(col, 0.22 * alpha));
            if media == Media::Audio {
                wave(p, r.shrink2(Vec2::new(7.0, 10.0)), seed, with_alpha(col, alpha), 7);
            } else {
                let (t, f) = fit_label(p, ext, r.width() - 6.0, 9.5, 6.5);
                p.text(r.center(), Align2::CENTER_CENTER, t, f, with_alpha(lighten(col, 0.15), alpha));
            }
        }
    }
}

/// Large art for files without a picture: a waveform or the file type.
fn draw_file_art(p: &Painter, r: Rect, media: Media, ext: &str, seed: u32, theme: &Theme) {
    let col = file_color(media, seed, theme);
    widgets::glow(p, r.center(), r.height() * 0.6, with_alpha(col, 0.18));
    if media == Media::Audio {
        wave(p, r.shrink2(Vec2::new(28.0, r.height() * 0.28)), seed, col, 48);
        return;
    }
    let doc = Rect::from_center_size(r.center(), Vec2::new(r.height() * 0.42, r.height() * 0.54));
    p.rect_filled(doc, cr(10.0), with_alpha(col, 0.25));
    p.rect_stroke(doc, cr(10.0), Stroke::new(1.5, with_alpha(col, 0.7)), egui::StrokeKind::Inside);
    let (t, f) = fit_label(p, ext, doc.width() - 14.0, 20.0, 11.0);
    p.text(doc.center(), Align2::CENTER_CENTER, t, f, lighten(col, 0.18));
}

fn wave(p: &Painter, r: Rect, seed: u32, col: Color32, bars: usize) {
    let bw = r.width() / bars as f32;
    for i in 0..bars {
        let t = i as f32 / bars as f32;
        let v = ((t * 9.0 + seed as f32 * 0.001).sin() * 0.5 + 0.5) * 0.6 + ((t * 31.0 + seed as f32 * 0.003).sin() * 0.5 + 0.5) * 0.4;
        let h = (r.height() * (0.15 + 0.85 * v)).max(2.0);
        let b = Rect::from_center_size(Pos2::new(r.left() + bw * (i as f32 + 0.5), r.center().y), Vec2::new((bw * 0.6).max(1.5), h));
        p.rect_filled(b, cr(bw * 0.3), col);
    }
}

fn file_color(media: Media, seed: u32, theme: &Theme) -> Color32 {
    match media {
        Media::Audio => theme.accent2,
        _ => theme.wheel_at((seed % 997) as f32 / 997.0),
    }
}

fn place_icon(kind: PlaceKind) -> Icon {
    match kind {
        PlaceKind::Folder => Icon::Folder,
        PlaceKind::Disk => Icon::External,
        PlaceKind::Server => Icon::Server,
        PlaceKind::Cloud => Icon::Cloud,
    }
}

/// Each place gets its own colour, so copies are easy to tell apart.
fn place_color(k: usize, theme: &Theme) -> Color32 {
    theme.wheel_at(0.08 + k as f32 * 0.29)
}

/// UV rect that crops an image of `img` aspect to fill a frame of `frame` aspect.
fn cover_uv(img: f32, frame: f32) -> Rect {
    if frame > img {
        let h = img / frame;
        Rect::from_min_max(Pos2::new(0.0, (1.0 - h) / 2.0), Pos2::new(1.0, (1.0 + h) / 2.0))
    } else {
        let w = frame / img;
        Rect::from_min_max(Pos2::new((1.0 - w) / 2.0, 0.0), Pos2::new((1.0 + w) / 2.0, 1.0))
    }
}

/// A file type label that fits `max_w`: smaller type first, then cut
/// (`.parquet`, `.fcpbundle`… in a small tile).
fn fit_label(p: &Painter, text: &str, max_w: f32, size: f32, min: f32) -> (String, egui::FontId) {
    let mut s = size;
    while s > min && p.layout_no_wrap(text.to_string(), bold(s), Color32::WHITE).size().x > max_w {
        s -= 0.5;
    }
    (widgets::truncate(p, text, &bold(s), max_w), bold(s))
}

/// Keeps the start and the end of a path: `/Volumes/T7/…/Shoot/footage`.
fn truncate_middle(p: &Painter, text: &str, font: &egui::FontId, max_w: f32) -> String {
    let w = |s: &str| p.layout_no_wrap(s.to_string(), font.clone(), Color32::WHITE).size().x;
    if w(text) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let cut = |keep: usize| -> String {
        let head = keep * 2 / 5;
        let tail = keep - head;
        chars[..head].iter().collect::<String>() + "\u{2026}" + &chars[chars.len() - tail..].iter().collect::<String>()
    };
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if w(&cut(mid)) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    cut(lo)
}

fn fmt_date((y, m, d): (u16, u8, u8)) -> String {
    format!("{y}-{m:02}-{d:02}")
}

fn fmt_elapsed(d: Duration) -> String {
    let s = d.as_secs_f32();
    if s < 60.0 {
        format!("{s:.1} s")
    } else {
        fmt_duration(s as u32)
    }
}

fn fmt_duration(s: u32) -> String {
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// A made-up landscape standing in for the photo, until real thumbnails.
fn fake_photo(seed: u32, w: usize, h: usize) -> egui::ColorImage {
    let f = |k: u32| (seed.wrapping_mul(2_654_435_761).rotate_left(k * 7) % 1000) as f32 / 1000.0;
    let hue = f(1);
    let sky_top = from_hsl(hue, 0.5, 0.3);
    let sky_bot = from_hsl((hue + 0.08) % 1.0, 0.7, 0.72);
    let sun_c = from_hsl((hue + 0.12) % 1.0, 0.95, 0.88);
    let far = from_hsl((hue + 0.6) % 1.0, 0.22, 0.42);
    let near = from_hsl((hue + 0.55) % 1.0, 0.28, 0.24);
    let ground = from_hsl((hue + 0.5) % 1.0, 0.3, 0.14);
    let (sx, sy) = (0.18 + 0.64 * f(2), 0.2 + 0.2 * f(3));
    let (p1, p2, p3, p4) = (f(4) * 6.3, f(5) * 6.3, f(6) * 6.3, f(7) * 6.3);
    let aspect = w as f32 / h as f32;
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        let v = y as f32 / h as f32;
        for x in 0..w {
            let u = x as f32 / w as f32;
            let mut c = lerp_color(sky_top, sky_bot, (v / 0.68).min(1.0));
            let d = (((u - sx) * aspect).powi(2) + (v - sy).powi(2)).sqrt();
            if d < 0.3 {
                c = lerp_color(c, sun_c, (1.0 - (d - 0.075) / 0.225).clamp(0.0, 1.0).powi(3) * 0.5);
            }
            c = lerp_color(c, sun_c, ((0.075 - d) * h as f32 + 0.5).clamp(0.0, 1.0));
            let hf = 0.5 + 0.07 * (u * 7.0 + p1).sin() + 0.04 * (u * 17.0 + p2).sin();
            let hn = 0.66 + 0.06 * (u * 5.0 + p3).sin() + 0.025 * (u * 23.0 + p4).sin();
            // ridges blended over one pixel, so their edges are smooth
            let px_h = 1.0 / h as f32;
            let cover = |edge: f32| ((v - edge) / px_h + 0.5).clamp(0.0, 1.0);
            c = lerp_color(c, lerp_color(far, sky_bot, 0.15), cover(hf));
            c = lerp_color(c, lerp_color(near, ground, ((v - hn) / (1.0 - hn)).clamp(0.0, 1.0)), cover(hn));
            if v > 0.86 {
                // a lake mirroring the sky
                let m = 1.0 - (v - 0.86) / 0.14;
                c = lerp_color(ground, lerp_color(sky_bot, sky_top, 0.4), 0.55 * m + 0.1 * ((u * 90.0 + v * 400.0).sin() * 0.5 + 0.5));
            }
            px.push(c);
        }
    }
    egui::ColorImage::new([w, h], px)
}

// ---------------------------------------------------------------------------
// removing

impl App {
    /// "Remove N copies?": what goes where, and how.
    pub(super) fn dupes_confirm_modal(&mut self, ctx: &egui::Context, mut ack: bool, mut permanent: bool) -> Option<Modal> {
        let theme = self.theme.clone();
        let d = self.dupes.as_ref()?;
        let batches = d.removal();
        // per place: copies and bytes
        let mut per: Vec<(String, PlaceKind, usize, u64)> = Vec::new();
        for (k, pl) in d.places.iter().enumerate() {
            let (n, b) = d.found().flat_map(|g| g.copies.iter().filter(|c| c.place == k && c.marked && !c.kept()).map(move |c| if c.shares { 0 } else { g.size })).fold((0, 0), |(n, b), s| (n + 1, b + s));
            if n > 0 {
                per.push((pl.label.clone(), pl.kind, n, b));
            }
        }
        let count: usize = batches.iter().map(|b| b.remove.len()).sum();
        let bytes: u64 = batches.iter().map(|b| b.remove.iter().filter(|t| t.frees).count() as u64 * b.size).sum();
        let remote = batches.iter().flat_map(|b| &b.remove).any(|t| t.at != remove::Location::Local);
        let (mut close, mut go) = (false, false);
        let needs_ack = remote || permanent;
        let h = 290.0 + per.len().min(6) as f32 * 30.0 + if needs_ack { 40.0 } else { 0.0 } + if remote && !permanent { 24.0 } else { 0.0 };
        let bd = super::modals::modal_frame(ctx, self.modal_opened, Vec2::new(600.0, h), &theme, |ui, card| {
            let sub = trf("{size} will be freed", &[("size", &fmt_size(bytes))]);
            super::modals::title(ui, card, &trf("Remove {n} copies?", &[("n", &fmt_count(count as u64))]), Some(&sub), &theme);
            close |= super::modals::close_button(ui, card, &theme);
            let x0 = card.left() + 28.0;
            let w = card.width() - 56.0;
            let mut y = card.top() + 96.0;
            let p = ui.painter();
            for (label, kind, n, b) in per.iter().take(6) {
                widgets::draw_icon(p, place_icon(*kind), Rect::from_center_size(Pos2::new(x0 + 8.0, y + 10.0), Vec2::splat(14.0)), theme.text_dim);
                p.text(Pos2::new(x0 + 26.0, y + 10.0), Align2::LEFT_CENTER, label, bold(13.0), theme.text);
                p.text(Pos2::new(x0 + w, y + 10.0), Align2::RIGHT_CENTER, format!("{} · {}", trf("{n} copies", &[("n", n)]), fmt_size(*b)), font(12.5), theme.text_dim);
                y += 30.0;
            }
            y += 8.0;
            // how: to the Trash, or for good
            let cw = (w - 10.0) / 2.0;
            let a = Rect::from_min_size(Pos2::new(x0, y), Vec2::new(cw, 54.0));
            let b2 = Rect::from_min_size(Pos2::new(x0 + cw + 10.0, y), Vec2::new(cw, 54.0));
            if widgets::choice_card(ui, a, Id::new("dupes-mode-trash"), !permanent, tr("Move to Trash"), tr("Can be recovered"), Icon::Trash, &theme, true).clicked() {
                permanent = false;
            }
            if widgets::choice_card(ui, b2, Id::new("dupes-mode-perm"), permanent, tr("Delete permanently"), tr("Cannot be recovered"), Icon::Warning, &theme, true).clicked() {
                permanent = true;
            }
            y += 66.0;
            let note = if permanent {
                tr("The copies are deleted right away, without going to the Trash. One copy of every file always stays, and it is checked before anything is removed.")
            } else {
                tr("Files on this computer go to the Trash. One copy of every file always stays, and it is checked before anything is removed.")
            };
            y += super::modals::wrapped(ui, Pos2::new(x0, y), w, note, font(12.0), theme.text_dim) + 10.0;
            if remote && !permanent {
                let warn = tr("Servers and cloud storage have no Trash: copies there are deleted for good.");
                y += super::modals::wrapped(ui, Pos2::new(x0, y), w, warn, bold(12.0), theme.danger) + 8.0;
            }
            if needs_ack {
                widgets::checkbox(ui, Rect::from_min_size(Pos2::new(x0, y), Vec2::new(w, 24.0)), Id::new("dupes-ack"), &mut ack, tr("I understand that these copies cannot be recovered"), &theme);
            }
            let (label, icon) = if permanent { (tr("Delete permanently"), Icon::Warning) } else { (tr("Move to Trash"), Icon::Trash) };
            let b = Rect::from_min_size(Pos2::new(card.right() - 28.0 - 190.0, card.bottom() - 60.0), Vec2::new(190.0, 36.0));
            go |= widgets::button_ex(ui, b, Id::new("dupes-go"), label, Some(icon), BtnStyle::Danger, &theme, count > 0 && (ack || !needs_ack)).clicked();
            let c = Rect::from_min_size(Pos2::new(b.left() - 110.0, b.top()), Vec2::new(100.0, 36.0));
            close |= widgets::button(ui, c, Id::new("dupes-cancel"), tr("Cancel"), None, BtnStyle::Ghost, &theme).clicked();
        });
        if go {
            if let Some(d) = self.dupes.as_mut() {
                d.removing = Some(remove::start(batches, permanent));
            }
            self.sfx(Sfx::Crunch);
            return Some(Modal::DupesRemoving { finished_at: None });
        }
        if close || bd { None } else { Some(Modal::DupesConfirm { ack, permanent }) }
    }

    pub(super) fn dupes_removing_modal(&mut self, ctx: &egui::Context, mut finished_at: Option<Instant>) -> Option<Modal> {
        let theme = self.theme.clone();
        let job = self.dupes.as_ref().and_then(|d| d.removing.clone())?;
        let (total, done, freed) = (job.total.load(Ordering::Relaxed), job.done.load(Ordering::Relaxed), job.freed.load(Ordering::Relaxed));
        let finished = job.finished.load(Ordering::Relaxed);
        if finished && finished_at.is_none() {
            finished_at = Some(Instant::now());
            self.sfx(Sfx::Success);
        }
        let errors = job.errors.lock().unwrap().clone();
        let mut close = false;
        let h = if finished && !errors.is_empty() { 300.0 } else { 220.0 };
        modal_frame_static(ctx, self.modal_opened, Vec2::new(500.0, h), &theme, |ui, card| {
            let title = if finished { tr("Done") } else { tr("Removing copies…") };
            let sub = trf("{size} freed", &[("size", &fmt_size(freed))]);
            super::modals::title(ui, card, title, Some(&sub), &theme);
            let x0 = card.left() + 28.0;
            let w = card.width() - 56.0;
            let bar = Rect::from_min_size(Pos2::new(x0, card.top() + 104.0), Vec2::new(w, 8.0));
            widgets::progress_bar(ui.painter(), bar, Some(done as f32 / total.max(1) as f32), &theme, self.time, if finished { Some(theme.ok) } else { None });
            ui.painter().text(Pos2::new(x0, bar.bottom() + 16.0), Align2::LEFT_CENTER, trf("{done} of {total}", &[("done", &fmt_count(done)), ("total", &fmt_count(total))]), font(12.0), theme.text_dim);
            if finished && !errors.is_empty() {
                let mut text = trf("{n} could not be removed:", &[("n", &errors.len())]);
                for e in errors.iter().take(3) {
                    text += &format!("\n{e}");
                }
                super::modals::wrapped(ui, Pos2::new(x0, bar.bottom() + 36.0), w, &text, font(11.5), theme.warn);
            }
            if finished {
                let b = Rect::from_min_size(Pos2::new(card.right() - 28.0 - 110.0, card.bottom() - 60.0), Vec2::new(110.0, 36.0));
                close |= widgets::button(ui, b, Id::new("dupes-done"), tr("Close"), None, BtnStyle::Primary, &theme).clicked();
            } else {
                let b = Rect::from_min_size(Pos2::new(card.right() - 28.0 - 110.0, card.bottom() - 60.0), Vec2::new(110.0, 36.0));
                if widgets::button(ui, b, Id::new("dupes-stop-rm"), tr("Stop"), None, BtnStyle::Ghost, &theme).clicked() {
                    job.cancel.store(true, Ordering::Relaxed);
                }
            }
        });
        ctx.request_repaint_after(Duration::from_millis(100));
        if close {
            if let Some(d) = self.dupes.as_mut() {
                d.poll();
                d.removing = None;
            }
            if freed > 0 {
                self.toasts.push(trf("{size} freed", &[("size", &fmt_size(freed))]), theme.ok);
            }
            return None;
        }
        Some(Modal::DupesRemoving { finished_at })
    }
}

/// A modal that only closes from its own buttons.
fn modal_frame_static(ctx: &egui::Context, opened: Instant, size: Vec2, theme: &Theme, add: impl FnOnce(&mut Ui, Rect)) {
    let _ = super::modals::modal_frame(ctx, opened, size, theme, add);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removal_keeps_one_copy_and_libraries() {
        let mut d = DupesUi::new();
        d.add_place(place_for(&Target::Folder("/p/Pictures".into())));
        d.add_place(place_for(&Target::Folder("/p/Backup".into())));
        let f = |place, path: &str, mtime| FoundFile { place, path: path.into(), mtime, shares_data: false };
        d.merge(vec![(0, vec![
            FoundGroup { key: 1, size: 10, digest: "ab".into(), files: vec![f(0, "/p/Pictures/Photos Library.photoslibrary/originals/IMG_1.HEIC", 50), f(0, "/p/Pictures/Imports/IMG_1.HEIC", 10), f(1, "/p/Backup/IMG_1.HEIC", 20)] },
            FoundGroup { key: 2, size: 7, digest: "cd".into(), files: vec![f(0, "/p/Pictures/a.mov", 10), f(1, "/p/Backup/x/a.mov", 20)] },
        ])]);
        let batches = d.removal();
        assert_eq!(batches.len(), 2);
        let lib = batches.iter().find(|b| b.size == 10).unwrap();
        // the library copy is kept, even though it is the newest
        assert_eq!(lib.keep.len(), 1);
        assert!(lib.keep[0].path.contains(".photoslibrary"));
        assert_eq!(lib.remove.len(), 2);
        let mov = batches.iter().find(|b| b.size == 7).unwrap();
        assert_eq!(mov.keep[0].path, "/p/Pictures/a.mov", "oldest kept");
        // marking every copy by hand removes nothing from that group
        let gi = d.groups.iter().position(|g| g.size == 7).unwrap();
        d.toggle_copy(gi, 0);
        assert!(d.groups[gi].none_kept());
        assert!(d.removal().iter().all(|b| b.size != 7));
        // a later change to one size leaves the others, and choices made by hand
        let gi = d.groups.iter().position(|g| g.size == 10).unwrap();
        let manual_path = d.groups[gi].copies[1].path.clone();
        d.toggle_copy(gi, 1);
        let before = d.groups.iter().find(|g| g.size == 10).unwrap().copies.iter().find(|c| c.path == manual_path).unwrap().marked;
        d.merge(vec![(7, vec![])]);
        assert!(d.groups.iter().all(|g| g.size != 7));
        let g10 = d.groups.iter().find(|g| g.size == 10).unwrap();
        assert_eq!(g10.copies.iter().find(|c| c.path == manual_path).unwrap().marked, before);
        assert_eq!(d.view().order.len(), 1);
        let gi = d.groups.iter().position(|g| g.size == 10).unwrap();
        // shown with its whole path
        assert_eq!(DupesUi::shown(&d.groups[gi].copies[2], "IMG_1.HEIC"), "/p/Backup");
        assert_eq!(DupesUi::shown(&d.groups[gi].copies[2], "other.HEIC"), "/p/Backup/IMG_1.HEIC");
    }
}
