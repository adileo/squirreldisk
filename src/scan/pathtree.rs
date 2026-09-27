//! Builds a tree incrementally from flat `(path, size)` records.
//! Used by remote backends (`find`, `du`, `rclone lsjson`).

use super::local::SMALL_FILES_NAME;
use crate::tree::{Kind, Tree, F_DONE};
use std::collections::HashMap;

pub struct PathTreeBuilder {
    /// Relative dir path -> node id.
    dirs: HashMap<String, u32>,
    /// Dir node id -> its small files node.
    small: HashMap<u32, u32>,
    pub keep_min: u64,
}

impl PathTreeBuilder {
    pub fn new(root: u32, keep_min: u64) -> Self {
        let mut dirs = HashMap::new();
        dirs.insert(String::new(), root);
        PathTreeBuilder { dirs, small: HashMap::new(), keep_min }
    }

    /// Makes sure the directory `rel` (slash separated, relative) exists.
    pub fn ensure_dir(&mut self, tree: &mut Tree, rel: &str) -> u32 {
        let rel = rel.trim_matches('/');
        if let Some(id) = self.dirs.get(rel) {
            return *id;
        }
        let (parent_rel, name) = match rel.rfind('/') {
            Some(i) => (&rel[..i], &rel[i + 1..]),
            None => ("", rel),
        };
        let parent = self.ensure_dir(tree, parent_rel);
        let id = tree.add_child(parent, name, Kind::Dir, 0, 0);
        tree.get_mut(id).flags |= F_DONE;
        self.dirs.insert(rel.to_string(), id);
        id
    }

    pub fn add_file(&mut self, tree: &mut Tree, rel: &str, size: u64) {
        let rel = rel.trim_matches('/');
        let (parent_rel, name) = match rel.rfind('/') {
            Some(i) => (&rel[..i], &rel[i + 1..]),
            None => ("", rel),
        };
        let parent = self.ensure_dir(tree, parent_rel);
        if size >= self.keep_min {
            tree.add_child_sized(parent, name, Kind::File, size, 1);
        } else {
            let small = match self.small.get(&parent) {
                Some(s) => *s,
                None => {
                    let s = tree.add_child(parent, SMALL_FILES_NAME, Kind::SmallFiles, 0, 0);
                    self.small.insert(parent, s);
                    s
                }
            };
            tree.add_size(small, size as i64, 1);
        }
    }

    /// For `du`-style records (post-order cumulative directory totals): sets the
    /// directory's *own* files size as the difference with its children.
    pub fn add_du_dir(&mut self, tree: &mut Tree, rel: &str, cumulative: u64) {
        let id = self.ensure_dir(tree, rel);
        let current = tree.get(id).size;
        if cumulative > current {
            let small = tree.add_child(id, SMALL_FILES_NAME, Kind::SmallFiles, 0, 0);
            tree.add_size(small, (cumulative - current) as i64, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::Source;

    #[test]
    fn builds_from_paths() {
        let mut t = Tree::new("/", "/", Source::Ssh { host: "x".into() });
        let mut b = PathTreeBuilder::new(0, 100);
        b.add_file(&mut t, "a/b/c.bin", 1000);
        b.add_file(&mut t, "a/x.txt", 10);
        b.add_file(&mut t, "a/y.txt", 20);
        assert_eq!(t.get(0).size, 1030);
        let (a, _) = t.find_path("/a");
        assert_eq!(t.child_count(a), 2); // b + small files
    }

    #[test]
    fn du_records() {
        let mut t = Tree::new("/", "/", Source::Ssh { host: "x".into() });
        let mut b = PathTreeBuilder::new(0, 100);
        b.add_du_dir(&mut t, "a/b", 300);
        b.add_du_dir(&mut t, "a", 500);
        b.add_du_dir(&mut t, "", 600);
        assert_eq!(t.get(0).size, 600);
        let (a, _) = t.find_path("/a");
        assert_eq!(t.get(a).size, 500);
    }
}
