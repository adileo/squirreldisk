//! Headless agent mode, used on remote machines over SSH.
//!
//! Protocol (stdout, one record per line, tab separated):
//! * `P <files> <bytes> <expected>` – progress, repeated while scanning
//! * `N <depth> <kind> <size> <files> <flags> <name>` – tree nodes in pre-order,
//!   emitted once the scan completes. Names are escaped (`\t`, `\n`, `\\`).

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
        _ => {
            eprintln!("usage: squirreldisk --agent (version|scan <path>)");
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
