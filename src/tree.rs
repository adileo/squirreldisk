//! Compact arena-based file tree.
//!
//! Every directory is a node; files are only materialised as nodes when they are
//! "big enough" – the rest of a directory's files are folded into a single
//! `SmallFiles` node that can be expanded lazily. This keeps RAM usage bounded
//! even for disks with millions of files.

use std::collections::HashMap;

pub const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Kind {
    Dir,
    File,
    /// Aggregate of many small files of the parent directory.
    SmallFiles,
    /// Symbolic link / junction (never followed, counted as 0).
    #[allow(dead_code)]
    Link,
    /// Another filesystem mounted inside the scanned one (not descended).
    Mount,
    /// Space used on the volume that could not be attributed to any file.
    Hidden,
}

pub const F_DONE: u8 = 1; // directory fully listed
pub const F_DENIED: u8 = 2; // permission denied / unreadable
pub const F_DETACHED: u8 = 4; // removed from the tree (tombstone)
pub const F_EXPANDABLE: u8 = 8; // SmallFiles node that can be expanded from disk

#[derive(Clone, Debug)]
pub struct Node {
    pub name: Box<str>,
    /// Allocated bytes of the whole subtree.
    pub size: u64,
    /// Number of files in the subtree.
    pub files: u32,
    pub parent: u32,
    pub first_child: u32,
    pub next_sibling: u32,
    pub kind: Kind,
    pub flags: u8,
}

impl Node {
    pub fn denied(&self) -> bool {
        self.flags & F_DENIED != 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Local,
    Ssh { host: String },
    Rclone { remote: String },
    /// `MARKETING_DEMO`: fake data, nothing on disk.
    Demo,
}

impl Source {
    pub fn is_local(&self) -> bool {
        matches!(self, Source::Local)
    }
}

pub struct Tree {
    pub nodes: Vec<Node>,
    pub root: u32,
    /// Incremented on every mutation – used by the UI to invalidate caches.
    pub version: u64,
    /// Full path of the root (local path, remote path, or `remote:path`).
    pub root_path: String,
    pub sep: char,
    pub source: Source,
    detached: usize,
}

impl Tree {
    pub fn new(root_path: &str, display_name: &str, source: Source) -> Self {
        let sep = if source.is_local() { std::path::MAIN_SEPARATOR } else { '/' };
        let root = Node {
            name: display_name.into(),
            size: 0,
            files: 0,
            parent: NONE,
            first_child: NONE,
            next_sibling: NONE,
            kind: Kind::Dir,
            flags: 0,
        };
        Tree {
            nodes: vec![root],
            root: 0,
            version: 1,
            root_path: root_path.to_string(),
            sep,
            source,
            detached: 0,
        }
    }

    #[inline]
    pub fn get(&self, id: u32) -> &Node {
        &self.nodes[id as usize]
    }

    #[inline]
    pub fn get_mut(&mut self, id: u32) -> &mut Node {
        &mut self.nodes[id as usize]
    }

    pub fn is_alive(&self, id: u32) -> bool {
        if id == NONE || id as usize >= self.nodes.len() {
            return false;
        }
        // A node is alive if no ancestor (including itself) is detached.
        let mut cur = id;
        while cur != NONE {
            let n = self.get(cur);
            if n.flags & F_DETACHED != 0 {
                return false;
            }
            cur = n.parent;
        }
        true
    }

    /// Adds a child (without propagating any size – see [`Tree::add_size`]).
    pub fn add_child(&mut self, parent: u32, name: &str, kind: Kind, size: u64, files: u32) -> u32 {
        let id = self.nodes.len() as u32;
        let next = self.get(parent).first_child;
        self.nodes.push(Node {
            name: name.into(),
            size,
            files,
            parent,
            first_child: NONE,
            next_sibling: next,
            kind,
            flags: 0,
        });
        self.get_mut(parent).first_child = id;
        self.version += 1;
        id
    }

    /// Adds `size`/`files` to `id` and all of its ancestors.
    pub fn add_size(&mut self, id: u32, size: i64, files: i64) {
        let mut cur = id;
        while cur != NONE {
            let n = &mut self.nodes[cur as usize];
            n.size = (n.size as i64 + size).max(0) as u64;
            n.files = (n.files as i64 + files).max(0) as u32;
            cur = n.parent;
        }
        self.version += 1;
    }

    /// Adds a child that already carries a size and propagates it to the ancestors.
    pub fn add_child_sized(&mut self, parent: u32, name: &str, kind: Kind, size: u64, files: u32) -> u32 {
        let id = self.add_child(parent, name, kind, 0, 0);
        self.add_size(id, size as i64, files as i64);
        id
    }

    pub fn children(&self, id: u32) -> ChildIter<'_> {
        ChildIter { tree: self, cur: self.get(id).first_child }
    }

    pub fn child_count(&self, id: u32) -> usize {
        self.children(id).count()
    }

    /// Children sorted by size, largest first.
    pub fn sorted_children(&self, id: u32) -> Vec<u32> {
        let mut v: Vec<u32> = self.children(id).collect();
        v.sort_unstable_by(|a, b| self.get(*b).size.cmp(&self.get(*a).size));
        v
    }

    pub fn find_child(&self, id: u32, name: &str) -> Option<u32> {
        self.children(id).find(|c| &*self.get(*c).name == name)
    }

    /// Unlinks a node from its parent and subtracts its size from the ancestors.
    pub fn remove(&mut self, id: u32) {
        if id == self.root || !self.is_alive(id) {
            return;
        }
        let (parent, size, files) = {
            let n = self.get(id);
            (n.parent, n.size, n.files)
        };
        if parent != NONE {
            self.add_size(parent, -(size as i64), -(files as i64));
            // unlink
            let first = self.get(parent).first_child;
            if first == id {
                self.get_mut(parent).first_child = self.get(id).next_sibling;
            } else {
                let mut cur = first;
                while cur != NONE {
                    let next = self.get(cur).next_sibling;
                    if next == id {
                        self.get_mut(cur).next_sibling = self.get(id).next_sibling;
                        break;
                    }
                    cur = next;
                }
            }
        }
        let n = self.get_mut(id);
        n.flags |= F_DETACHED;
        n.next_sibling = NONE;
        self.detached += 1;
        self.version += 1;
    }

    /// Path segments of a node, below the root.
    pub fn rel_components(&self, id: u32) -> Vec<&str> {
        let mut parts = Vec::new();
        let mut cur = id;
        while cur != NONE && cur != self.root {
            parts.push(&*self.get(cur).name);
            cur = self.get(cur).parent;
        }
        parts.reverse();
        parts
    }

    /// Full path of a node as a string.
    pub fn path(&self, id: u32) -> String {
        let mut s = self.root_path.clone();
        for p in self.rel_components(id) {
            if !s.ends_with(self.sep) && !s.ends_with(':') {
                s.push(self.sep);
            }
            s.push_str(p);
        }
        s
    }

    pub fn path_buf(&self, id: u32) -> std::path::PathBuf {
        std::path::PathBuf::from(self.path(id))
    }

    /// Chain of ancestors from root to `id` (inclusive).
    pub fn ancestry(&self, id: u32) -> Vec<u32> {
        let mut v = Vec::new();
        let mut cur = id;
        while cur != NONE {
            v.push(cur);
            cur = self.get(cur).parent;
        }
        v.reverse();
        v
    }

    pub fn is_ancestor(&self, anc: u32, id: u32) -> bool {
        let mut cur = self.get(id).parent;
        while cur != NONE {
            if cur == anc {
                return true;
            }
            cur = self.get(cur).parent;
        }
        false
    }

    /// Finds the node for a full path. Returns the deepest existing node and
    /// whether it is an exact match.
    pub fn find_path(&self, path: &str) -> (u32, bool) {
        let root = self.root_path.trim_end_matches(self.sep);
        let rest = if let Some(r) = path.strip_prefix(root) { r } else { return (NONE, false) };
        if !rest.is_empty() && !rest.starts_with(self.sep) && !root.is_empty() {
            return (NONE, false);
        }
        let mut cur = self.root;
        for comp in rest.split(self.sep).filter(|c| !c.is_empty()) {
            match self.find_child(cur, comp) {
                Some(c) => cur = c,
                None => return (cur, false),
            }
        }
        (cur, true)
    }

    /// Copies another tree's root subtree under `parent` (used for incremental rescans).
    pub fn graft(&mut self, parent: u32, other: &Tree, name: &str) -> u32 {
        let mut map: HashMap<u32, u32> = HashMap::new();
        let o_root = other.get(other.root);
        let new_root = self.add_child(parent, name, o_root.kind, o_root.size, o_root.files);
        self.get_mut(new_root).flags = o_root.flags;
        map.insert(other.root, new_root);
        let mut stack = vec![other.root];
        while let Some(o) = stack.pop() {
            let dst = map[&o];
            for c in other.children(o) {
                let cn = other.get(c);
                let id = self.add_child(dst, &cn.name, cn.kind, cn.size, cn.files);
                self.get_mut(id).flags = cn.flags;
                map.insert(c, id);
                stack.push(c);
            }
        }
        let (s, f) = (o_root.size, o_root.files);
        if parent != NONE {
            self.add_size(parent, s as i64, f as i64);
        }
        new_root
    }
}

pub struct ChildIter<'a> {
    tree: &'a Tree,
    cur: u32,
}

impl Iterator for ChildIter<'_> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        if self.cur == NONE {
            return None;
        }
        let id = self.cur;
        self.cur = self.tree.get(id).next_sibling;
        Some(id)
    }
}

/// Human-readable size, e.g. `438.1 GB`
pub fn fmt_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    if v >= 100.0 {
        format!("{v:.0} {}", UNITS[u])
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// Compact count for values that change quickly (live scan progress):
/// `999`, `2k`, `125k`, `1.75M`.
pub fn fmt_count_compact(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{}k", n / 1_000)
    } else {
        format!("{:.2}M", (n / 10_000) as f64 / 100.0)
    }
}

pub fn fmt_count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_propagate_and_remove() {
        let mut t = Tree::new("/r", "r", Source::Local);
        let a = t.add_child(0, "a", Kind::Dir, 0, 0);
        let b = t.add_child_sized(a, "b", Kind::File, 100, 1);
        let _c = t.add_child_sized(0, "c", Kind::File, 50, 1);
        assert_eq!(t.get(0).size, 150);
        assert_eq!(t.get(a).size, 100);
        t.remove(b);
        assert_eq!(t.get(0).size, 50);
        assert_eq!(t.get(a).size, 0);
        assert!(!t.is_alive(b));
        assert_eq!(t.child_count(a), 0);
    }

    #[test]
    fn paths() {
        let mut t = Tree::new("/r", "r", Source::Ssh { host: "h".into() });
        let a = t.add_child(0, "a", Kind::Dir, 0, 0);
        let b = t.add_child(a, "b", Kind::Dir, 0, 0);
        assert_eq!(t.path(b), "/r/a/b");
        assert_eq!(t.find_path("/r/a/b"), (b, true));
        assert_eq!(t.find_path("/r/a/x"), (a, false));
    }

    #[test]
    fn format() {
        assert_eq!(fmt_size(438_100_000_000), "438 GB");
        assert_eq!(fmt_size(17_400_000_000), "17.4 GB");
        assert_eq!(fmt_count(1234567), "1,234,567");
        assert_eq!(fmt_count_compact(999), "999");
        assert_eq!(fmt_count_compact(2_345), "2k");
        assert_eq!(fmt_count_compact(1_758_000), "1.75M");
    }
}
