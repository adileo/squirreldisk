#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![cfg_attr(not(feature = "gui"), allow(dead_code))]

mod agent;
mod delete;
mod rclone;
mod safety;
mod scan;
mod settings;
mod tree;

#[cfg(feature = "gui")]
mod disks;
#[cfg(feature = "gui")]
mod i18n;
#[cfg(feature = "gui")]
mod icon;
#[cfg(feature = "gui")]
mod sound;
#[cfg(feature = "gui")]
mod sponsor;
#[cfg(feature = "gui")]
mod ui;
#[cfg(feature = "gui")]
mod update;
#[cfg(feature = "gui")]
mod watch;

/// GitHub repository used for releases, self-update and agent downloads.
pub const GITHUB_REPO: &str = "adileo/squirreldisk";

fn main() {
    // Invoked by ssh as SSH_ASKPASS helper: answer the prompt and leave.
    if let Ok(secret) = std::env::var(scan::remote::ASKPASS_ENV) {
        println!("{secret}");
        return;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|a| a == "--agent").unwrap_or(false) {
        std::process::exit(agent::main(&args[1..]));
    }
    #[cfg(feature = "gui")]
    if args.first().map(|a| a == "--render-icons").unwrap_or(false) {
        let dir = std::path::PathBuf::from(args.get(1).map(|s| s.as_str()).unwrap_or("assets/icon"));
        match icon::write_assets(&dir) {
            Ok(()) => println!("icons written to {}", dir.display()),
            Err(e) => {
                eprintln!("could not write icons: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    if args.first().map(|a| a == "--version" || a == "-V").unwrap_or(false) {
        println!("squirreldisk {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    #[cfg(feature = "gui")]
    run_gui();
    #[cfg(not(feature = "gui"))]
    {
        eprintln!("This is the headless SquirrelDisk agent. Usage: squirreldisk --agent (version|scan <path>)");
        std::process::exit(2);
    }
}

#[cfg(feature = "gui")]
fn run_gui() {
    use eframe::egui;
    let icon = app_icon();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("SquirrelDisk")
        .with_inner_size([1180.0, 760.0])
        .with_min_inner_size([860.0, 560.0])
        .with_drag_and_drop(true)
        .with_icon(icon);
    // SQD_WINDOW=WxH (development: check layouts at a given size)
    if let Some((w, h)) = std::env::var("SQD_WINDOW").ok().and_then(|v| {
        let (w, h) = v.split_once('x')?;
        Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?))
    }) {
        viewport = viewport.with_inner_size([w, h]);
    }
    if cfg!(target_os = "macos") {
        viewport = viewport.with_fullsize_content_view(true).with_titlebar_shown(false).with_title_shown(false);
    }
    let options = eframe::NativeOptions { viewport, multisampling: 0, ..Default::default() };
    if let Err(e) = eframe::run_native("SquirrelDisk", options, Box::new(|cc| Ok(Box::new(ui::app::App::new(cc))))) {
        eprintln!("SquirrelDisk failed to start: {e}");
        std::process::exit(1);
    }
}

/// Window icon, rendered from the same code as every other icon.
#[cfg(feature = "gui")]
fn app_icon() -> eframe::egui::IconData {
    let pm = icon::render(256, icon::Frame::IDLE, false);
    eframe::egui::IconData { rgba: icon::rgba(&pm), width: 256, height: 256 }
}
