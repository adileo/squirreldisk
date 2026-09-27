//! Remote scanning over SSH (agent / GNU find / du) and cloud storage via rclone.

use super::pathtree::PathTreeBuilder;
use super::{Progress, ScanHandle, SharedTree};
use crate::tree::{Kind, Source, Tree, F_DONE, NONE};
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

pub const AGENT_DIR: &str = "~/.cache/squirreldisk";

pub fn agent_path() -> String {
    format!("{AGENT_DIR}/agent-{}", env!("CARGO_PKG_VERSION"))
}

/// Quotes a string for a POSIX shell.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Env var through which the app hands a passphrase/password to itself when
/// ssh runs it as `SSH_ASKPASS` helper (see `main.rs`).
pub const ASKPASS_ENV: &str = "SQUIRRELDISK_ASKPASS_SECRET";

fn secrets() -> &'static std::sync::Mutex<std::collections::HashMap<String, String>> {
    static S: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, String>>> = std::sync::OnceLock::new();
    S.get_or_init(Default::default)
}

/// Remembers (in memory only) the key passphrase or password for a host.
pub fn set_secret(host: &str, secret: String) {
    secrets().lock().unwrap().insert(host.to_string(), secret);
}

/// Does this ssh error mean we need a passphrase/password from the user?
pub fn is_auth_error(e: &str) -> bool {
    let e = e.to_lowercase();
    e.contains("permission denied") || e.contains("passphrase") || e.contains("too many authentication failures")
}

/// Common options for ssh/scp: connection sharing (authenticate once, reuse for
/// every later command) and, when the user gave us a secret, askpass.
pub fn apply_ssh_options(c: &mut Command, host: &str) {
    c.args(["-o", "ConnectTimeout=12", "-o", "StrictHostKeyChecking=accept-new"]);
    #[cfg(unix)]
    c.args(["-o", "ControlMaster=auto", "-o", "ControlPath=/tmp/sqd-ssh-%C", "-o", "ControlPersist=600"]);
    let secret = secrets().lock().unwrap().get(host).cloned();
    match secret {
        Some(secret) => {
            c.args(["-o", "BatchMode=no", "-o", "NumberOfPasswordPrompts=1"]);
            if let Ok(exe) = std::env::current_exe() {
                c.env("SSH_ASKPASS", exe);
            }
            c.env("SSH_ASKPASS_REQUIRE", "force");
            c.env("DISPLAY", std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into()));
            c.env(ASKPASS_ENV, secret);
        }
        None => {
            c.args(["-o", "BatchMode=yes"]);
        }
    }
    hide_console(c);
}

pub fn ssh_command(host: &str) -> Command {
    let mut c = Command::new("ssh");
    apply_ssh_options(&mut c, host);
    c.args(["-C", host]);
    c
}

#[allow(unused_variables)]
pub fn hide_console(c: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
}

pub fn run_ssh(host: &str, script: &str) -> Result<String, String> {
    let out = ssh_command(host).arg(script).stdin(Stdio::null()).output().map_err(|e| format!("cannot run ssh: {e}"))?;
    if !out.status.success() && out.stdout.is_empty() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { "ssh failed".into() } else { err });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Maps `uname -sm` to a Rust target triple used for release assets.
pub fn target_for_uname(uname: &str) -> Option<&'static str> {
    let u = uname.to_lowercase();
    let arm = u.contains("arm64") || u.contains("aarch64");
    let x64 = u.contains("x86_64") || u.contains("amd64");
    if u.starts_with("linux") {
        if arm {
            return Some("aarch64-unknown-linux-musl");
        }
        if x64 {
            return Some("x86_64-unknown-linux-musl");
        }
    } else if u.starts_with("darwin") {
        if arm {
            return Some("aarch64-apple-darwin");
        }
        if x64 {
            return Some("x86_64-apple-darwin");
        }
    }
    None
}

fn local_matches(target: &str) -> bool {
    let os_ok = (cfg!(target_os = "macos") && target.contains("apple"))
        || (cfg!(target_os = "linux") && target.contains("linux"));
    let arch_ok = (cfg!(target_arch = "aarch64") && target.starts_with("aarch64"))
        || (cfg!(target_arch = "x86_64") && target.starts_with("x86_64"));
    os_ok && arch_ok
}

enum Method {
    Agent,
    GnuFind,
    Du,
}

/// Prepares the remote host: installs the agent if possible, else picks a fallback.
fn prepare_ssh(host: &str, progress: &Progress) -> Result<Method, String> {
    progress.set_status("Connecting");
    let agent = agent_path();
    let probe = format!(
        "uname -sm; if [ -x {agent} ]; then echo HAVE_AGENT; fi; if find --version >/dev/null 2>&1; then echo HAVE_GNUFIND; fi; if command -v curl >/dev/null 2>&1; then echo HAVE_CURL; fi"
    );
    let out = run_ssh(host, &probe)?;
    let mut lines = out.lines();
    let uname = lines.next().unwrap_or("").to_string();
    let flags: Vec<&str> = lines.collect();
    if flags.contains(&"HAVE_AGENT") {
        return Ok(Method::Agent);
    }
    if let Some(target) = target_for_uname(&uname) {
        // 1) try the official release asset
        if flags.contains(&"HAVE_CURL") {
            progress.set_status("Installing agent");
            let url = format!(
                "https://github.com/{}/releases/download/v{}/squirreldisk-agent-{}",
                crate::GITHUB_REPO,
                env!("CARGO_PKG_VERSION"),
                target
            );
            let script = format!(
                "mkdir -p {AGENT_DIR} && curl -fsSL {url} -o {agent}.tmp && chmod +x {agent}.tmp && {agent}.tmp --agent version >/dev/null && mv {agent}.tmp {agent} && echo OK"
            );
            if run_ssh(host, &script).map(|o| o.contains("OK")).unwrap_or(false) {
                return Ok(Method::Agent);
            }
        }
        // 2) same platform: upload ourselves
        if local_matches(target) {
            progress.set_status("Uploading agent");
            if upload_self(host).is_ok() {
                return Ok(Method::Agent);
            }
        }
    }
    if flags.contains(&"HAVE_GNUFIND") {
        Ok(Method::GnuFind)
    } else {
        Ok(Method::Du)
    }
}

fn upload_self(host: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let agent = agent_path();
    let script = format!(
        "mkdir -p {AGENT_DIR} && cat > {agent}.tmp && chmod +x {agent}.tmp && {agent}.tmp --agent version >/dev/null && mv {agent}.tmp {agent} && echo OK"
    );
    let mut child = ssh_command(host)
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    {
        let mut stdin = child.stdin.take().unwrap();
        let mut f = std::fs::File::open(exe).map_err(|e| e.to_string())?;
        std::io::copy(&mut f, &mut stdin).map_err(|e| e.to_string())?;
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if String::from_utf8_lossy(&out.stdout).contains("OK") {
        Ok(())
    } else {
        Err("upload failed".into())
    }
}

/// Reads a child's stdout line by line, applying `f` in batches under the tree lock.
fn pump_lines(
    mut child: Child,
    tree: &SharedTree,
    progress: &Progress,
    mut f: impl FnMut(&mut Tree, &str, &Progress),
) -> Result<(), String> {
    let stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take();
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(e) = stderr.as_mut() {
            let _ = e.read_to_string(&mut s);
        }
        s
    });
    let reader = BufReader::with_capacity(256 * 1024, stdout);
    let mut batch: Vec<String> = Vec::with_capacity(4096);
    let mut last = Instant::now();
    let flush = |batch: &mut Vec<String>, f: &mut dyn FnMut(&mut Tree, &str, &Progress)| {
        let mut t = tree.write().unwrap();
        for l in batch.drain(..) {
            f(&mut t, &l, progress);
        }
    };
    for line in reader.split(b'\n') {
        if progress.cancelled() {
            let _ = child.kill();
            return Err("cancelled".into());
        }
        let Ok(line) = line else { break };
        batch.push(String::from_utf8_lossy(&line).into_owned());
        if batch.len() >= 4096 || last.elapsed() > Duration::from_millis(120) {
            flush(&mut batch, &mut f);
            last = Instant::now();
        }
    }
    flush(&mut batch, &mut f);
    let status = child.wait().map_err(|e| e.to_string())?;
    let err = err_thread.join().unwrap_or_default();
    let has_data = tree.read().unwrap().get(0).size > 0;
    if !status.success() && !has_data {
        return Err(if err.trim().is_empty() { format!("remote command failed ({status})") } else { err.trim().to_string() });
    }
    Ok(())
}

/// Parses one line of the agent protocol (see `agent.rs`).
pub struct AgentParser {
    stack: Vec<u32>,
}

impl AgentParser {
    pub fn new() -> Self {
        AgentParser { stack: Vec::new() }
    }
    pub fn line(&mut self, t: &mut Tree, line: &str, p: &Progress) {
        let mut it = line.split('\t');
        match it.next() {
            Some("P") => {
                let files = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                let bytes = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                p.files.store(files, Ordering::Relaxed);
                p.bytes.store(bytes, Ordering::Relaxed);
                if let Some(e) = it.next().and_then(|s| s.parse().ok()) {
                    p.expected.store(e, Ordering::Relaxed);
                }
            }
            Some("N") => {
                let depth: usize = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                let kind = match it.next().unwrap_or("d") {
                    "f" => Kind::File,
                    "s" => Kind::SmallFiles,
                    "m" => Kind::Mount,
                    "h" => Kind::Hidden,
                    _ => Kind::Dir,
                };
                let size: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                let files: u32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                let flags: u8 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                let name = unescape(it.next().unwrap_or(""));
                if depth == 0 {
                    let r = t.root;
                    let n = t.get_mut(r);
                    n.size = size;
                    n.files = files;
                    n.flags = flags;
                    self.stack = vec![r];
                    t.version += 1;
                    return;
                }
                self.stack.truncate(depth);
                let parent = *self.stack.last().unwrap_or(&t.root);
                let id = t.add_child(parent, &name, kind, size, files);
                t.get_mut(id).flags = flags & !crate::tree::F_EXPANDABLE;
                self.stack.push(id);
            }
            _ => {}
        }
    }
}

pub fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n")
}

pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn start_ssh(host: String, path: String) -> ScanHandle {
    let progress = Arc::new(Progress::default());
    let display = format!("{host}:{path}");
    let tree = Arc::new(RwLock::new(Tree::new(&path, &display, Source::Ssh { host: host.clone() })));
    let (t, p) = (tree.clone(), progress.clone());
    std::thread::spawn(move || {
        let res = (|| -> Result<(), String> {
            let method = prepare_ssh(&host, &p)?;
            p.set_status("Scanning");
            let q = sh_quote(&path);
            match method {
                Method::Agent => {
                    let child = ssh_command(&host)
                        .arg(format!("{} --agent scan {q}", agent_path()))
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .stdin(Stdio::null())
                        .spawn()
                        .map_err(|e| e.to_string())?;
                    let mut parser = AgentParser::new();
                    pump_lines(child, &t, &p, |tr, l, pr| parser.line(tr, l, pr))
                }
                Method::GnuFind => {
                    let child = ssh_command(&host)
                        .arg(format!("find {q} -xdev \\( -type f -o -type d \\) -printf '%y\\t%b\\t%n\\t%i\\t%P\\n' 2>/dev/null"))
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .stdin(Stdio::null())
                        .spawn()
                        .map_err(|e| e.to_string())?;
                    let mut b = PathTreeBuilder::new(0, 256 * 1024);
                    let mut seen = std::collections::HashSet::new();
                    pump_lines(child, &t, &p, |tr, l, pr| {
                        let mut it = l.splitn(5, '\t');
                        let (Some(ty), Some(blocks), Some(nlink), Some(ino), Some(rel)) =
                            (it.next(), it.next(), it.next(), it.next(), it.next())
                        else {
                            return;
                        };
                        let mut size = blocks.parse::<u64>().unwrap_or(0) * 512;
                        if ty == "d" {
                            if !rel.is_empty() {
                                b.ensure_dir(tr, rel);
                            }
                            pr.dirs.fetch_add(1, Ordering::Relaxed);
                            return;
                        }
                        if nlink != "1" && !seen.insert(ino.to_string()) {
                            size = 0;
                        }
                        pr.files.fetch_add(1, Ordering::Relaxed);
                        pr.bytes.fetch_add(size, Ordering::Relaxed);
                        b.add_file(tr, rel, size);
                    })
                }
                Method::Du => {
                    let child = ssh_command(&host)
                        .arg(format!("du -xk {q} 2>/dev/null"))
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .stdin(Stdio::null())
                        .spawn()
                        .map_err(|e| e.to_string())?;
                    let mut b = PathTreeBuilder::new(0, u64::MAX);
                    let prefix = path.trim_end_matches('/').to_string();
                    pump_lines(child, &t, &p, |tr, l, pr| {
                        let Some((kb, full)) = l.split_once('\t') else { return };
                        let size = kb.trim().parse::<u64>().unwrap_or(0) * 1024;
                        let rel = full.strip_prefix(&prefix).unwrap_or(full);
                        b.add_du_dir(tr, rel, size);
                        pr.dirs.fetch_add(1, Ordering::Relaxed);
                        pr.bytes.store(tr.get(0).size, Ordering::Relaxed);
                    })
                }
            }
        })();
        if let Err(e) = res {
            p.fail(e);
        } else {
            let mut tr = t.write().unwrap();
            let r = tr.root;
            tr.get_mut(r).flags |= F_DONE;
            p.bytes.store(tr.get(r).size, Ordering::Relaxed);
            p.set_status("Done");
            p.done.store(true, Ordering::Relaxed);
        }
    });
    ScanHandle { tree, progress }
}

// ---------------------------------------------------------------------------
// rclone (S3, Google Drive, FTP, SFTP, Dropbox, OneDrive, ...)

pub fn rclone_available() -> bool {
    let mut c = Command::new("rclone");
    hide_console(&mut c);
    c.arg("version").stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

pub fn rclone_remotes() -> Vec<String> {
    let mut c = Command::new("rclone");
    hide_console(&mut c);
    match c.arg("listremotes").output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect(),
        Err(_) => Vec::new(),
    }
}

#[derive(serde::Deserialize)]
struct LsEntry {
    #[serde(rename = "Path")]
    path: String,
    #[serde(rename = "Size", default)]
    size: i64,
}

pub fn start_rclone(remote_path: String) -> ScanHandle {
    let progress = Arc::new(Progress::default());
    let tree = Arc::new(RwLock::new(Tree::new(&remote_path, &remote_path, Source::Rclone { remote: remote_path.clone() })));
    let (t, p) = (tree.clone(), progress.clone());
    std::thread::spawn(move || {
        p.set_status("Listing");
        let mut c = Command::new("rclone");
        hide_console(&mut c);
        let child = c
            .args(["lsjson", "-R", "--files-only", "--fast-list", "--no-mimetype", "--no-modtime", &remote_path])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .spawn();
        let res = match child {
            Err(e) => Err(format!("cannot run rclone: {e}")),
            Ok(child) => {
                let mut b = PathTreeBuilder::new(0, 256 * 1024);
                pump_lines(child, &t, &p, |tr, l, pr| {
                    let l = l.trim().trim_end_matches(',');
                    if !l.starts_with('{') {
                        return;
                    }
                    if let Ok(e) = serde_json::from_str::<LsEntry>(l) {
                        let size = e.size.max(0) as u64;
                        b.add_file(tr, &e.path, size);
                        pr.files.fetch_add(1, Ordering::Relaxed);
                        pr.bytes.fetch_add(size, Ordering::Relaxed);
                    }
                })
            }
        };
        match res {
            Err(e) => p.fail(e),
            Ok(()) => {
                t.write().unwrap().get_mut(0).flags |= F_DONE;
                p.set_status("Done");
                p.done.store(true, Ordering::Relaxed);
            }
        }
    });
    let _ = NONE;
    ScanHandle { tree, progress }
}

/// Lists `Host` aliases from ~/.ssh/config for convenience.
pub fn ssh_config_hosts() -> Vec<String> {
    let Some(home) = crate::settings::home_dir() else { return Vec::new() };
    let Ok(s) = std::fs::read_to_string(home.join(".ssh").join("config")) else { return Vec::new() };
    let mut v = Vec::new();
    for line in s.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("Host ").or_else(|| l.strip_prefix("host ")) {
            for h in rest.split_whitespace() {
                if !h.contains('*') && !h.contains('?') && !v.contains(&h.to_string()) {
                    v.push(h.to_string());
                }
            }
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(sh_quote("a'b"), "'a'\\''b'");
        assert_eq!(unescape(&escape("a\tb\\c\nd")), "a\tb\\c\nd");
    }

    #[test]
    fn agent_protocol_roundtrip() {
        let mut t = Tree::new("/", "/", Source::Ssh { host: "h".into() });
        let p = Progress::default();
        let mut parser = AgentParser::new();
        for l in ["N\t0\td\t300\t2\t1\t/", "N\t1\td\t200\t1\t1\tusr", "N\t2\tf\t200\t1\t0\tbig", "N\t1\tf\t100\t1\t0\tx"] {
            parser.line(&mut t, l, &p);
        }
        assert_eq!(t.get(0).size, 300);
        let (big, ok) = t.find_path("/usr/big");
        assert!(ok);
        assert_eq!(t.get(big).size, 200);
    }
}
