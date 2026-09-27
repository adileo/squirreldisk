//! On-device interest inference.
//!
//! Works only on **folder names that are already in memory** after a scan and
//! on their **sizes**. It never opens a file, never looks at file contents,
//! and the result is a small set of broad categories kept in RAM only.
//!
//! A category counts when the folders that signal it add up to at least 1 GB
//! (or 3% of what was scanned, minimum 200 MB).

use crate::tree::{Kind, Tree, NONE};
use crate::i18n::tr;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interest {
    Developer,
    Creator,
    Gamer,
    Music,
    Design,
    Cloud,
    Virtualization,
    Ai,
}

impl Interest {
    pub const ALL: [Interest; 8] = [
        Interest::Developer,
        Interest::Creator,
        Interest::Gamer,
        Interest::Music,
        Interest::Design,
        Interest::Cloud,
        Interest::Virtualization,
        Interest::Ai,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Interest::Developer => tr("software development"),
            Interest::Creator => tr("photo & video"),
            Interest::Gamer => tr("gaming"),
            Interest::Music => tr("music production"),
            Interest::Design => tr("design"),
            Interest::Cloud => tr("cloud storage"),
            Interest::Virtualization => tr("virtual machines"),
            Interest::Ai => tr("local AI models"),
        }
    }

    fn bit(self) -> u16 {
        1 << (self as u16)
    }
}

/// Folder-name markers. Exact (case-insensitive) names or `*.suffix`.
const MARKERS: &[(&str, Interest)] = &[
    ("node_modules", Interest::Developer),
    (".cargo", Interest::Developer),
    (".rustup", Interest::Developer),
    (".npm", Interest::Developer),
    (".gradle", Interest::Developer),
    (".m2", Interest::Developer),
    (".pyenv", Interest::Developer),
    ("DerivedData", Interest::Developer),
    ("Xcode.app", Interest::Developer),
    ("Docker.app", Interest::Developer),
    ("com.docker.docker", Interest::Developer),
    ("Android", Interest::Developer),
    ("*.photoslibrary", Interest::Creator),
    ("*.fcpbundle", Interest::Creator),
    ("*.lrdata", Interest::Creator),
    ("Adobe Lightroom Classic", Interest::Creator),
    ("Adobe Premiere Pro 2025", Interest::Creator),
    ("DaVinci Resolve", Interest::Creator),
    ("Final Cut Pro.app", Interest::Creator),
    ("Capture One", Interest::Creator),
    ("steamapps", Interest::Gamer),
    ("Steam", Interest::Gamer),
    ("Epic Games", Interest::Gamer),
    ("Battle.net", Interest::Gamer),
    ("GOG Galaxy", Interest::Gamer),
    ("Riot Games", Interest::Gamer),
    ("Audio Music Apps", Interest::Music),
    ("Logic Pro.app", Interest::Music),
    ("Ableton", Interest::Music),
    ("GarageBand.app", Interest::Music),
    ("Native Instruments", Interest::Music),
    ("Splice", Interest::Music),
    ("Figma.app", Interest::Design),
    ("Sketch.app", Interest::Design),
    ("Affinity Designer 2.app", Interest::Design),
    ("Adobe Illustrator 2025", Interest::Design),
    ("Adobe Photoshop 2025", Interest::Design),
    ("Blender.app", Interest::Design),
    ("CloudStorage", Interest::Cloud),
    ("Dropbox", Interest::Cloud),
    ("Google Drive", Interest::Cloud),
    ("OneDrive", Interest::Cloud),
    ("Mobile Documents", Interest::Cloud),
    ("Parallels", Interest::Virtualization),
    ("Virtual Machines.localized", Interest::Virtualization),
    ("*.utm", Interest::Virtualization),
    ("*.vmwarevm", Interest::Virtualization),
    ("VirtualBox VMs", Interest::Virtualization),
    (".ollama", Interest::Ai),
    (".lmstudio", Interest::Ai),
    ("huggingface", Interest::Ai),
];

fn classify(name: &str) -> Option<Interest> {
    for (m, i) in MARKERS {
        let hit = match m.strip_prefix('*') {
            Some(suffix) => name.len() > suffix.len() && name[name.len() - suffix.len()..].eq_ignore_ascii_case(suffix),
            None => name.eq_ignore_ascii_case(m),
        };
        if hit {
            return Some(*i);
        }
    }
    None
}

/// Returns the categories with enough bytes behind them.
pub fn infer(tree: &Tree) -> Vec<Interest> {
    let mut bytes = [0u64; 8];
    // DFS carrying which categories an ancestor already claimed, so nested
    // markers (node_modules inside node_modules) are counted once.
    let mut stack: Vec<(u32, u16)> = vec![(tree.root, 0)];
    while let Some((id, claimed)) = stack.pop() {
        let n = tree.get(id);
        let mut claimed = claimed;
        if let Some(i) = classify(&n.name) {
            if claimed & i.bit() == 0 {
                bytes[i as usize] += n.size;
                claimed |= i.bit();
            }
        }
        if n.kind == Kind::Dir && n.first_child != NONE {
            for c in tree.children(id) {
                if tree.get(c).kind == Kind::Dir {
                    stack.push((c, claimed));
                }
            }
        }
    }
    let total = tree.get(tree.root).size.max(1);
    let min = (total * 3 / 100).clamp(200_000_000, 1_000_000_000);
    Interest::ALL.iter().copied().filter(|i| bytes[*i as usize] >= min).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::Source;

    #[test]
    fn infers_from_folder_names_and_sizes() {
        let mut t = Tree::new("/", "/", Source::Local);
        let proj = t.add_child(0, "projects", Kind::Dir, 0, 0);
        let nm = t.add_child(proj, "node_modules", Kind::Dir, 0, 0);
        let nested = t.add_child(nm, "node_modules", Kind::Dir, 0, 0);
        t.add_child_sized(nested, "big.bin", Kind::File, 2_000_000_000, 1);
        let games = t.add_child(0, "Steam", Kind::Dir, 0, 0);
        t.add_child_sized(games, "tiny", Kind::File, 10_000, 1);
        let got = infer(&t);
        assert_eq!(got, vec![Interest::Developer]); // nested counted once, Steam too small
        assert_eq!(classify("Library.photoslibrary"), Some(Interest::Creator));
    }
}
