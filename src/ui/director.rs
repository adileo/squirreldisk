//! Scripted demo "director" used to record the README screenshots and GIFs.
//!
//! Only active with `MARKETING_DEMO=1` and `MARKETING_DEMO_SCRIPT=<name>`.
//! It drives the real UI by injecting pointer events (so hover, clicks and
//! drag & drop behave exactly like with a mouse), draws a cursor, and with
//! `MARKETING_DEMO_RECORD=<dir>` saves frames at 15 fps.

use super::app::{App, Screen, Target};
use super::sunburst::View;
use eframe::egui::{self, Color32, Event, Id, LayerId, Order, PointerButton, Pos2, Shape, Stroke, Vec2};
use std::sync::mpsc::Sender;

pub const FPS: f64 = 15.0;

#[derive(Clone, Debug)]
pub enum Aim {
    /// A sunburst slice, by path relative to the scan root.
    Node(&'static str),
    /// A rectangle registered by the UI with `App::mark`.
    Mark(&'static str),
    /// Fraction of the window.
    Frac(f32, f32),
}

#[derive(Clone, Debug)]
pub enum Step {
    Wait(f64),
    Move(Aim, f64),
    Down,
    Up,
    Scan(&'static str),
    WaitScan,
    Navigate(&'static str),
    Theme(&'static str),
    Record(bool),
}

use Step::*;

fn click() -> Vec<Step> {
    vec![Down, Wait(0.09), Up]
}

fn script(name: &str) -> Vec<Step> {
    let mut s = Vec::new();
    match name {
        // Home → scan → live chart → hover → zoom in → back out
        "hero" => {
            s.extend([Record(true), Wait(0.9), Move(Aim::Mark("scan:/"), 0.9), Wait(0.25)]);
            s.extend(click());
            s.extend([Wait(0.3), Move(Aim::Frac(0.52, 0.93), 1.2), Wait(5.4), Move(Aim::Node("Users/alex/Movies"), 1.0), Wait(1.1)]);
            s.extend(click());
            s.extend([Wait(1.3), Move(Aim::Node("Users/alex/Movies/Final Cut Library.fcpbundle"), 0.8), Wait(1.2)]);
            s.extend([Move(Aim::Frac(0.315, 0.475), 0.8), Wait(0.3)]);
            s.extend(click());
            s.extend([Wait(1.4)]);
        }
        // Drag slices into the collector, then delete them
        "collect" => {
            s.extend([Scan("/"), WaitScan, Navigate("Users/alex"), Wait(1.2), Record(true), Wait(0.6)]);
            s.extend([Move(Aim::Node("Users/alex/Downloads"), 0.9), Wait(0.3), Down, Wait(0.15), Move(Aim::Mark("bin"), 1.0), Wait(0.2), Up, Wait(0.7)]);
            s.extend([Move(Aim::Node("Users/alex/Library/Caches"), 0.9), Wait(0.3), Down, Wait(0.15), Move(Aim::Mark("bin"), 1.0), Wait(0.2), Up, Wait(0.8)]);
            s.extend([Move(Aim::Mark("delete"), 0.8), Wait(0.2)]);
            s.extend(click());
            s.extend([Wait(1.0), Move(Aim::Mark("mode-1"), 0.7), Wait(0.2)]);
            s.extend(click());
            s.extend([Wait(0.5), Move(Aim::Mark("del-go"), 0.7), Wait(0.3)]);
            s.extend(click());
            s.extend([Wait(3.2)]);
        }
        // Themes
        "themes" => {
            s.extend([Scan("/"), WaitScan, Wait(1.0), Record(true), Wait(0.6), Move(Aim::Mark("settings"), 0.9), Wait(0.2)]);
            s.extend(click());
            for t in ["theme:Midnight Neon", "theme:Autumn Forest", "theme:Paper", "theme:Sunset"] {
                s.extend([Wait(0.5), Move(Aim::Mark(t), 0.6), Wait(0.15)]);
                s.extend(click());
            }
            s.extend([Wait(0.6), Move(Aim::Mark("modal-close"), 0.7), Wait(0.2)]);
            s.extend(click());
            s.extend([Wait(1.6), Theme("Hazelnut")]);
        }
        _ => {}
    }
    s
}

pub struct Director {
    steps: Vec<Step>,
    idx: usize,
    step_start: f64,
    from: Pos2,
    pub pos: Pos2,
    pub pressed: bool,
    events: Vec<Event>,
    pub recording: bool,
    next_frame: f64,
    frame_no: usize,
    writer: Option<Sender<(usize, std::sync::Arc<egui::ColorImage>)>>,
    finished_at: Option<f64>,
}

impl Director {
    pub fn from_env() -> Option<Director> {
        if !crate::scan::demo::enabled() {
            return None;
        }
        let name = std::env::var("MARKETING_DEMO_SCRIPT").ok()?;
        let steps = script(&name);
        if steps.is_empty() {
            return None;
        }
        let writer = std::env::var("MARKETING_DEMO_RECORD").ok().map(|dir| {
            let _ = std::fs::create_dir_all(&dir);
            let (tx, rx) = std::sync::mpsc::channel::<(usize, std::sync::Arc<egui::ColorImage>)>();
            std::thread::spawn(move || {
                for (n, img) in rx {
                    let _ = write_frame(&format!("{dir}/frame-{n:05}.png"), &img);
                }
            });
            tx
        });
        Some(Director {
            steps,
            idx: 0,
            step_start: 0.0,
            from: Pos2::new(600.0, 420.0),
            pos: Pos2::new(600.0, 420.0),
            pressed: false,
            events: Vec::new(),
            recording: false,
            next_frame: 0.0,
            frame_no: 0,
            writer,
            finished_at: None,
        })
    }

    /// Replaces the real pointer input with the scripted one.
    pub fn inject(&mut self, raw: &mut egui::RawInput) {
        raw.events.retain(|e| {
            !matches!(e, Event::PointerMoved(_) | Event::PointerButton { .. } | Event::PointerGone | Event::MouseWheel { .. })
        });
        raw.events.push(Event::PointerMoved(self.pos));
        raw.events.append(&mut self.events);
    }
}

fn ease(t: f64) -> f32 {
    let t = t.clamp(0.0, 1.0);
    (if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }) as f32
}

/// Writes a frame as PNG at native resolution (ffmpeg scales it later).
fn write_frame(path: &str, img: &egui::ColorImage) -> Result<(), String> {
    let (w, h) = (img.size[0], img.size[1]);
    let mut data = Vec::with_capacity(w * h * 4);
    for p in &img.pixels {
        data.extend_from_slice(&[p.r(), p.g(), p.b(), 255]);
    }
    let size = tiny_skia::IntSize::from_wh(w as u32, h as u32).ok_or("size")?;
    let pm = tiny_skia::Pixmap::from_vec(data, size).ok_or("pixmap")?;
    let png = pm.encode_png().map_err(|e| e.to_string())?;
    std::fs::write(path, png).map_err(|e| e.to_string())
}

impl App {
    fn aim_pos(&self, aim: &Aim, ctx: &egui::Context) -> Pos2 {
        let screen = ctx.content_rect();
        match aim {
            Aim::Frac(x, y) => Pos2::new(screen.width() * x, screen.height() * y),
            Aim::Mark(name) => self.marks.get(*name).map(|r| r.center()).unwrap_or(screen.center()),
            Aim::Node(rel) => {
                let Screen::Session(i) = self.screen else { return screen.center() };
                let s = &self.sessions[i];
                let Some(geo) = s.geo else { return screen.center() };
                let t = s.tree.read().unwrap();
                let full = format!("{}/{}", t.root_path.trim_end_matches('/'), rel);
                let (id, exact) = t.find_path(&full);
                if !exact {
                    return screen.center();
                }
                match s.anim.anims.get(&(id as u64)) {
                    Some(a) => {
                        let (r0, r1) = geo.radii(a.d);
                        let (a0, a1) = if a.d < 0.5 { (0.0, 0.0) } else { (a.a0, a.a1) };
                        geo.point((r0 + r1) / 2.0, (a0 + a1) / 2.0)
                    }
                    None => screen.center(),
                }
            }
        }
    }

    /// Advances the script; call once per frame before drawing.
    pub fn direct(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.director.take() else { return };
        let now = self.time;
        if d.idx == 0 && d.step_start == 0.0 {
            d.step_start = now;
        }
        loop {
            let Some(step) = d.steps.get(d.idx).cloned() else {
                let end = *d.finished_at.get_or_insert(now);
                if now - end > 0.6 && d.writer.is_some() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                break;
            };
            let elapsed = now - d.step_start;
            let done = match &step {
                Wait(s) => elapsed >= *s,
                Move(aim, dur) => {
                    let target = self.aim_pos(aim, ctx);
                    let k = ease(elapsed / dur);
                    d.pos = d.from + (target - d.from) * k;
                    elapsed >= *dur
                }
                Down | Up => {
                    let pressed = matches!(step, Down);
                    d.pressed = pressed;
                    d.events.push(Event::PointerButton { pos: d.pos, button: PointerButton::Primary, pressed, modifiers: Default::default() });
                    true
                }
                Scan(mount) => {
                    if let Some(v) = self.volumes.iter().find(|v| v.mount == *mount).cloned() {
                        self.start_session(Target::Volume(v));
                    }
                    true
                }
                WaitScan => match self.screen {
                    Screen::Session(i) => !self.sessions[i].is_scanning() && now - d.step_start > 0.5,
                    _ => true,
                },
                Navigate(rel) => {
                    if let Screen::Session(i) = self.screen {
                        let s = &mut self.sessions[i];
                        let (id, ok) = {
                            let t = s.tree.read().unwrap();
                            t.find_path(&format!("{}/{}", t.root_path.trim_end_matches('/'), rel))
                        };
                        if ok {
                            s.navigate(View::node(id));
                        }
                    }
                    true
                }
                Theme(name) => {
                    self.theme = super::theme::by_name(name);
                    self.settings.theme = name.to_string();
                    true
                }
                Record(on) => {
                    d.recording = *on;
                    d.next_frame = now;
                    true
                }
            };
            if !done {
                break;
            }
            d.idx += 1;
            d.step_start = now;
            d.from = d.pos;
        }
        // frame capture
        if d.recording && d.writer.is_some() && now >= d.next_frame {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            d.next_frame += 1.0 / FPS;
            if d.next_frame < now {
                d.next_frame = now + 1.0 / FPS;
            }
        }
        let shots: Vec<std::sync::Arc<egui::ColorImage>> = ctx.input(|i| {
            i.raw.events.iter().filter_map(|e| if let Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }).collect()
        });
        if let Some(w) = &d.writer {
            for img in shots {
                let _ = w.send((d.frame_no, img));
                d.frame_no += 1;
            }
        }
        draw_cursor(ctx, d.pos, d.pressed);
        ctx.request_repaint();
        self.director = Some(d);
    }
}

/// macOS-style arrow cursor (screenshots don't include the system cursor).
fn draw_cursor(ctx: &egui::Context, pos: Pos2, pressed: bool) {
    let p = ctx.layer_painter(LayerId::new(Order::Debug, Id::new("demo-cursor")));
    let s = if pressed { 0.9 } else { 1.0 };
    let pts: Vec<Pos2> = [(0.0, 0.0), (0.0, 17.0), (4.2, 13.2), (7.2, 19.8), (9.8, 18.7), (6.9, 12.2), (12.4, 12.2)]
        .iter()
        .map(|(x, y)| pos + Vec2::new(*x, *y) * s)
        .collect();
    let shadow: Vec<Pos2> = pts.iter().map(|q| *q + Vec2::new(0.8, 1.2)).collect();
    p.add(Shape::convex_polygon(shadow, Color32::from_black_alpha(70), Stroke::NONE));
    p.add(Shape::closed_line(pts.clone(), Stroke::new(2.4, Color32::WHITE)));
    // fill as two convex pieces (arrow head + tail)
    p.add(Shape::convex_polygon(vec![pts[0], pts[1], pts[2], pts[5], pts[6]], Color32::BLACK, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![pts[2], pts[3], pts[4], pts[5]], Color32::BLACK, Stroke::NONE));
}
