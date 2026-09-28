//! Developer hooks driven by environment variables (used for automated
//! screenshots while developing; inert otherwise).
//!
//! * `SQD_SCAN=<path>`       start scanning a folder at launch
//! * `SQD_SHOT=<prefix>`     save screenshots as `<prefix>-<n>.bmp`
//! * `SQD_SHOT_AT=3,8`       seconds after launch at which to shoot
//! * `SQD_MODAL=settings|ssh|sshauth|rclone|delete` open a modal before shooting
//! * `SQD_COLLECT=2`         collect the N biggest children of the root
//! * `SQD_QUIT=1`            quit after the last screenshot

use super::app::{App, Modal, Target};
use eframe::egui;
use std::path::PathBuf;

#[derive(Default)]
pub struct Debug {
    shots: Vec<f64>,
    prefix: Option<String>,
    taken: usize,
    started: bool,
    pending: bool,
    modal_done: bool,
    collect_done: bool,
}

impl Debug {
    pub fn from_env() -> Self {
        let shots = std::env::var("SQD_SHOT_AT")
            .ok()
            .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
            .unwrap_or_else(|| vec![3.0]);
        Debug { shots, prefix: std::env::var("SQD_SHOT").ok(), ..Default::default() }
    }
}

impl App {
    pub fn debug_hooks(&mut self, ctx: &egui::Context) {
        if !self.debug.started {
            self.debug.started = true;
            if let Ok(p) = std::env::var("SQD_SCAN") {
                if let Some(v) = self.volumes.iter().find(|v| v.mount == p).cloned() {
                    self.start_session(Target::Volume(v));
                } else {
                    self.start_session(Target::Folder(PathBuf::from(p)));
                }
            }
        }
        if !self.debug.collect_done {
            if let (Ok(n), Some(s)) = (std::env::var("SQD_COLLECT"), self.sessions.first_mut()) {
                if s.progress.is_done() {
                    self.debug.collect_done = true;
                    let n: usize = n.parse().unwrap_or(1);
                    let kids: Vec<u32> = s.tree.read().unwrap().sorted_children(0).into_iter().take(n).collect();
                    for k in kids {
                        s.collect(k);
                    }
                    // Test-only auto delete, restricted to fake folders.
                    let fake = std::env::var("SQD_SCAN").map(|p| p.contains("sqd-fake")).unwrap_or(false);
                    if fake && std::env::var("SQD_AUTODELETE").is_ok() {
                        self.open_delete_confirm(0);
                        if let Some(Modal::ConfirmDelete { items, .. }) = self.modal.take() {
                            self.start_delete(0, items, crate::delete::Mode::Permanent);
                        }
                    }
                }
            } else if std::env::var("SQD_COLLECT").is_err() {
                self.debug.collect_done = true;
            }
        }
        let Some(prefix) = self.debug.prefix.clone() else { return };
        if !self.debug.modal_done && self.time > self.debug.shots.first().copied().unwrap_or(3.0) - 1.0 {
            self.debug.modal_done = true;
            match std::env::var("SQD_MODAL").as_deref() {
                Ok("settings") => self.open_modal(Modal::Settings),
                Ok("languages") => {
                    self.open_modal(Modal::Settings);
                    self.lang_picker = true;
                    super::app::install_fonts(ctx, crate::i18n::current(), true);
                }
                Ok("ssh") => self.open_modal(Modal::Ssh { host: String::new(), path: "/".into() }),
                Ok("sshauth") => self.open_modal(Modal::SshAuth { host: "user@server".into(), path: "/".into(), secret: String::new(), retry: false }),
                Ok("rclone") => self.open_modal(Modal::Rclone { path: String::new() }),
                Ok("delete") => {
                    if !self.sessions.is_empty() {
                        self.open_delete_confirm(0)
                    }
                }
                _ => {}
            }
        }
        // collect screenshot replies
        let images: Vec<std::sync::Arc<egui::ColorImage>> = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None })
                .collect()
        });
        for img in images {
            let path = format!("{prefix}-{}.bmp", self.debug.taken);
            let _ = write_bmp(&path, &img);
            let _ = std::process::Command::new("sips").args(["-s", "format", "png", &path, "--out", &path.replace(".bmp", ".png")]).output();
            let _ = std::fs::remove_file(&path);
            self.debug.taken += 1;
            if self.debug.taken >= self.debug.shots.len() && std::env::var("SQD_QUIT").is_ok() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        let requested = self.debug.shots.iter().filter(|t| self.time >= **t).count();
        if requested > self.debug.taken && requested <= self.debug.shots.len() && !self.debug.pending {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.debug.pending = true;
        }
        if self.debug.taken == requested {
            self.debug.pending = false;
        }
        ctx.request_repaint();
    }
}

fn write_bmp(path: &str, img: &egui::ColorImage) -> std::io::Result<()> {
    let (w, h) = (img.size[0] as u32, img.size[1] as u32);
    let data_len = w * h * 4;
    let mut out = Vec::with_capacity(54 + data_len as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + data_len).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(-(h as i32)).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for p in &img.pixels {
        out.extend_from_slice(&[p.b(), p.g(), p.r(), 255]);
    }
    std::fs::write(path, out)
}
