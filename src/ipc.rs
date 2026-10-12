//! The open app answers `squirreldisk scan` from the scans it already has,
//! so the command line does not walk the disk again. One request per
//! connection on a socket in the config folder: a line of JSON in, the text
//! `scan` prints out. No answer means: scan the folder yourself.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// In a folder of its own, closed to other users: the answers hold the names
/// and sizes of this user's files.
pub fn socket_path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("run").join("app.sock"))
}

/// The open app's answer to `request`, or None when no app answers or its
/// scans do not cover the request.
pub fn ask(socket: &Path, request: &str) -> Option<String> {
    let mut s = UnixStream::connect(socket).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    s.write_all(format!("{request}\n").as_bytes()).ok()?;
    let mut out = String::new();
    s.read_to_string(&mut out).ok()?;
    (!out.is_empty()).then_some(out)
}

/// A finished scan the window holds.
#[cfg(feature = "gui")]
pub struct Scan {
    pub tree: crate::scan::SharedTree,
    /// Kept current by the file system watcher.
    pub watched: bool,
    pub finished: std::time::Instant,
}

/// The finished scans of local folders, kept current by the app.
#[cfg(feature = "gui")]
#[derive(Default)]
pub struct Scans(pub Vec<Scan>);

#[cfg(feature = "gui")]
impl Scans {
    /// Replaces the list, keeping the time each scan was first seen finished.
    pub fn set(&mut self, now: impl Iterator<Item = (crate::scan::SharedTree, bool)>) {
        let old = std::mem::take(&mut self.0);
        self.0 = now
            .map(|(tree, watched)| {
                let finished = old.iter().find(|s| std::sync::Arc::ptr_eq(&s.tree, &tree)).map(|s| s.finished);
                Scan { tree, watched, finished: finished.unwrap_or_else(std::time::Instant::now) }
            })
            .collect();
    }
}

/// Answers each connection on `socket` with `answer(request)`, on a thread.
/// Does nothing when another SquirrelDisk window answers there already.
#[cfg(feature = "gui")]
pub fn serve(socket: PathBuf, answer: impl Fn(&str) -> String + Send + 'static) -> std::io::Result<()> {
    use std::io::{BufRead, BufReader};
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    if UnixStream::connect(&socket).is_ok() {
        return Ok(());
    }
    let _ = std::fs::remove_file(&socket); // left by an app that did not quit cleanly
    if let Some(dir) = socket.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let listener = std::os::unix::net::UnixListener::bind(&socket)?;
    std::thread::Builder::new().name("ipc".into()).spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_ok() {
                let _ = stream.write_all(answer(line.trim_end()).as_bytes());
            }
        }
    })?;
    Ok(())
}

#[cfg(all(test, feature = "gui"))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc, Mutex};

    #[test]
    fn the_open_app_answers_scan() {
        let dir = crate::cli::absolute(&std::env::temp_dir()).join(format!("sqd-ipc-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("big")).unwrap();
        std::fs::write(dir.join("big/a.bin"), vec![1u8; 400_000]).unwrap();
        std::fs::write(dir.join("small.bin"), vec![1u8; 50_000]).unwrap();
        let handle = crate::scan::local::start(dir.clone(), "x".into());
        while !handle.progress.is_done() {
            std::thread::sleep(Duration::from_millis(10));
        }
        let scans = Arc::new(Mutex::new(Scans::default()));
        scans.lock().unwrap().set([(handle.tree.clone(), false)].into_iter());
        // a socket path must fit in about 100 bytes: not under the long temp folder
        let run = PathBuf::from(format!("/tmp/sqd-ipc-{}", std::process::id()));
        let socket = run.join("app.sock");
        let s = scans.clone();
        serve(socket.clone(), move |r| crate::cli::answer(&s, r)).unwrap();
        assert_eq!(std::fs::metadata(&run).unwrap().permissions().mode() & 0o777, 0o700);
        let req = |path: &Path, json: bool| serde_json::json!({ "path": path, "depth": 1, "top": 20, "json": json }).to_string();

        let text = ask(&socket, &req(&dir.join("big"), false)).expect("a folder in the scan");
        assert!(text.starts_with(&dir.join("big").to_string_lossy().into_owned()), "{text}");
        assert!(text.contains("a.bin") && text.contains("from the open app, scanned"), "{text}");
        let json: serde_json::Value = serde_json::from_str(&ask(&socket, &req(&dir, true)).unwrap()).unwrap();
        assert_eq!(json["children"][0]["name"], "big");
        assert_eq!(json["from_app"]["watched"], false);
        // watched scans are current: no age to tell
        scans.lock().unwrap().set([(handle.tree.clone(), true)].into_iter());
        assert!(ask(&socket, &req(&dir, false)).unwrap().lines().next().unwrap().ends_with("from the open app"));
        // not in any scan, or not a folder: the command line scans by itself
        assert_eq!(ask(&socket, &req(Path::new("/nowhere/else"), false)), None);
        assert_eq!(ask(&socket, &req(&dir.join("big/a.bin"), false)), None);
        assert_eq!(ask(&socket, "not json"), None);

        let _ = std::fs::remove_dir_all(run);
        let _ = std::fs::remove_dir_all(dir);
    }
}
