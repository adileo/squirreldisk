// The app is a GUI program on Windows; the headless build stays a console
// one (it becomes `squirreldisk.com`, see `console_shim`).
#![cfg_attr(all(windows, feature = "gui", not(debug_assertions)), windows_subsystem = "windows")]
#![cfg_attr(not(feature = "gui"), allow(dead_code))]

mod agent;
mod cli;
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
    #[cfg(windows)]
    if let Some(code) = console_shim(&args) {
        std::process::exit(code);
    }
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
    if cli::handles(&args) {
        std::process::exit(cli::main(&args));
    }
    // Anything else opens the app, scanning the folder given if any (from a
    // terminal, the file manager's menu or `open --args`; macOS may add a
    // `-psn_…` argument, hence skipping flags).
    let folder = args.iter().find(|a| !a.starts_with('-')).map(|a| cli::absolute(std::path::Path::new(a)));
    if let Some(f) = folder.as_ref().filter(|f| !f.is_dir()) {
        eprintln!("squirreldisk: {}: not a folder\n\n{}", f.display(), cli::USAGE);
        std::process::exit(2);
    }
    #[cfg(feature = "gui")]
    {
        #[cfg(unix)]
        detach_from_terminal(&args);
        run_gui(folder);
    }
    #[cfg(not(feature = "gui"))]
    {
        let _ = folder;
        eprintln!("This is the headless SquirrelDisk build: it has no window.\n\n{}", cli::USAGE);
        std::process::exit(2);
    }
}

/// `squirreldisk.com`, installed next to `squirreldisk.exe` and on the PATH:
/// typed in a terminal, `squirreldisk` finds it first (PATHEXT lists .COM
/// before .EXE). The .exe is a GUI program, which terminals neither wait for
/// nor show the output of; this console build runs it with the terminal's
/// handles instead, waiting for it only for command-line work. All the logic
/// stays in the .exe, which the updater keeps current.
#[cfg(windows)]
fn console_shim(args: &[String]) -> Option<i32> {
    use std::process::{Command, Stdio};
    let me = std::env::current_exe().ok()?;
    if !me.extension()?.eq_ignore_ascii_case("com") {
        return None;
    }
    let exe = me.with_extension("exe");
    let mut cmd = Command::new(&exe);
    cmd.args(args);
    let fail = |e: std::io::Error| {
        eprintln!("squirreldisk: can't run {}: {e}", exe.display());
        1
    };
    Some(if cli::handles(args) || args.first().is_some_and(|a| a == "--agent") {
        cmd.status().map(|s| s.code().unwrap_or(1)).unwrap_or_else(fail)
    } else {
        // opening the app: leave the terminal free
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        cmd.spawn().map(|_| 0).unwrap_or_else(fail)
    })
}

/// `squirreldisk [folder]` typed in a terminal: start the app on its own
/// and give the terminal back, as other GUI apps do.
#[cfg(all(unix, feature = "gui"))]
fn detach_from_terminal(args: &[String]) {
    use std::io::IsTerminal;
    use std::os::unix::process::CommandExt;
    const DETACHED: &str = "SQUIRRELDISK_DETACHED";
    if !std::io::stdin().is_terminal() || std::env::var_os(DETACHED).is_some() {
        return;
    }
    let Ok(me) = std::env::current_exe().and_then(std::fs::canonicalize) else { return };
    // macOS: through Launch Services, so it starts as the app it is (Dock
    // icon, focus) even when typed via the /usr/local/bin link
    #[cfg(target_os = "macos")]
    if let Some(app) = me.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")) {
        let status = std::process::Command::new("open").arg("-n").arg("-a").arg(app).arg("--args").args(args).status();
        if status.is_ok_and(|s| s.success()) {
            std::process::exit(0);
        }
    }
    let mut cmd = std::process::Command::new(me);
    cmd.args(args).env(DETACHED, "1").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    // SAFETY: setsid is async-signal-safe, as required between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    if cmd.spawn().is_ok() {
        std::process::exit(0);
    }
}

#[cfg(feature = "gui")]
fn run_gui(folder: Option<std::path::PathBuf>) {
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
    // centred at the default size; the app then sizes it to the screen
    let options = eframe::NativeOptions { viewport, multisampling: 0, centered: true, ..Default::default() };
    if let Err(e) = eframe::run_native("SquirrelDisk", options, Box::new(|cc| {
        let mut app = ui::app::App::new(cc);
        app.open_at_start = folder;
        Ok(Box::new(app))
    })) {
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
