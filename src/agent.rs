//! Headless agent mode, used on remote machines over SSH.
//!
//! Protocol (stdout, one record per line, tab separated):
//! * `P <files> <bytes> <expected>` – progress, repeated while scanning
//! * `N <depth> <kind> <size> <files> <flags> <name>` – tree nodes in pre-order,
//!   emitted once the scan completes. Names are escaped (`\t`, `\n`, `\\`).
//!
//! Duplicate finder:
//! * `dupes-list <path> <min size>` prints `F <size> <mtime> <dev> <ino> <nlink> <path>`
//!   per file, `P <files> <bytes>` while listing and `D` at the end.
//! * `dupes-hash` reads `<kind> <size> <path>` lines and answers each with
//!   `H <hex>` or `X <error>` (kinds in [`crate::dupes::hash::Kind`]).

use crate::scan::local;
use crate::scan::remote::escape;
use crate::tree::{Kind, Tree};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

pub fn main(args: &[String]) -> i32 {
    match args.first().map(|s| s.as_str()) {
        Some("version") => {
            println!("squirreldisk-agent {}", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("scan") => {
            let Some(path) = args.get(1) else {
                eprintln!("usage: --agent scan <path>");
                return 2;
            };
            scan(PathBuf::from(path))
        }
        Some("dupes-list") => {
            let (Some(path), min) = (args.get(1), args.get(2).and_then(|m| m.parse().ok()).unwrap_or(64 * 1024)) else {
                eprintln!("usage: --agent dupes-list <path> [min size]");
                return 2;
            };
            dupes_list(PathBuf::from(path), min)
        }
        Some("dupes-hash") => dupes_hash(),
        _ => {
            eprintln!("usage: squirreldisk --agent (version|scan <path>|dupes-list <path> [min size]|dupes-hash)");
            2
        }
    }
}

fn scan(path: PathBuf) -> i32 {
    let name = path.to_string_lossy().into_owned();
    let handle = local::start(path, name);
    let out = std::io::stdout();
    let mut out = std::io::BufWriter::with_capacity(1 << 20, out.lock());
    loop {
        let p = &handle.progress;
        let done = p.is_done();
        let _ = writeln!(
            out,
            "P\t{}\t{}\t{}",
            p.files.load(Ordering::Relaxed),
            p.bytes.load(Ordering::Relaxed),
            p.expected.load(Ordering::Relaxed)
        );
        let _ = out.flush();
        if done {
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    let tree = handle.tree.read().unwrap();
    dump(&tree, &mut out);
    let _ = out.flush();
    0
}

pub fn dump(tree: &Tree, out: &mut impl Write) {
    let mut stack = vec![(tree.root, 0usize)];
    while let Some((id, depth)) = stack.pop() {
        let n = tree.get(id);
        let k = match n.kind {
            Kind::Dir => 'd',
            Kind::File => 'f',
            Kind::SmallFiles => 's',
            Kind::Mount => 'm',
            Kind::Hidden => 'h',
            Kind::Link => 'l',
        };
        let _ = writeln!(out, "N\t{depth}\t{k}\t{}\t{}\t{}\t{}", n.size, n.files, n.flags, escape(&n.name));
        for c in tree.children(id) {
            stack.push((c, depth + 1));
        }
    }
}

fn dupes_list(path: PathBuf, min_size: u64) -> i32 {
    use crate::dupes::walk::{walk, Counters};
    use std::sync::atomic::AtomicBool;
    use std::sync::Mutex;
    if !path.is_dir() {
        eprintln!("{} is not a folder", path.display());
        return 1;
    }
    let out = Mutex::new(std::io::BufWriter::with_capacity(1 << 20, std::io::stdout()));
    let (cancel, counters, done) = (AtomicBool::new(false), Counters::default(), AtomicBool::new(false));
    std::thread::scope(|s| {
        s.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(300));
                let mut o = out.lock().unwrap();
                let _ = writeln!(o, "P\t{}\t{}", counters.files.load(Ordering::Relaxed), counters.bytes.load(Ordering::Relaxed));
                let _ = o.flush();
            }
        });
        walk(&path, min_size, crate::scan::local::thread_count(), &cancel, &counters, &|f| {
            let mut o = out.lock().unwrap();
            let _ = writeln!(o, "F\t{}\t{}\t{}\t{}\t{}\t{}", f.size, f.mtime, f.dev, f.ino, f.nlink, escape(&f.path));
        });
        done.store(true, Ordering::Relaxed);
    });
    let mut o = out.lock().unwrap();
    let _ = writeln!(o, "P\t{}\t{}", counters.files.load(Ordering::Relaxed), counters.bytes.load(Ordering::Relaxed));
    let _ = writeln!(o, "D");
    let _ = o.flush();
    0
}

fn dupes_hash() -> i32 {
    use crate::dupes::hash::{file, to_hex, Kind};
    use crate::scan::remote::unescape;
    use std::io::BufRead;
    use std::sync::atomic::{AtomicBool, AtomicU64};
    let (cancel, read) = (AtomicBool::new(false), AtomicU64::new(0));
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let mut it = line.splitn(3, '\t');
        let (Some(kind), Some(size), Some(path)) = (it.next().and_then(Kind::from_code), it.next().and_then(|s| s.parse::<u64>().ok()), it.next()) else {
            let _ = writeln!(out, "X\tbad request");
            let _ = out.flush();
            continue;
        };
        let _ = match file(std::path::Path::new(&unescape(path)), kind, size, &cancel, &read) {
            Ok(h) => writeln!(out, "H\t{}", to_hex(&h)),
            Err(e) => writeln!(out, "X\t{}", escape(&e)),
        };
        let _ = out.flush();
    }
    0
}
