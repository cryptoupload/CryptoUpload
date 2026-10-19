// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;

use cu_vfs::DirRef;

pub type Ino = u64;

pub const ROOT_INO: Ino = 1;

enum Node {
    Dir {
        dir_ref: DirRef,
        parent: Ino,
        name: String,
    },
    File {
        parent: Ino,
        name: String,
    },
}

pub struct InodeTable {
    nodes: HashMap<Ino, Node>,
    by_parent_name: HashMap<(Ino, String), Ino>,
    next_ino: Ino,
}

impl InodeTable {
    #[must_use]
    pub fn new(root: DirRef) -> Self {
        let mut nodes = HashMap::new();
        nodes.insert(
            ROOT_INO,
            Node::Dir {
                dir_ref: root,
                parent: ROOT_INO,
                name: String::new(),
            },
        );
        Self {
            nodes,
            by_parent_name: HashMap::new(),
            next_ino: ROOT_INO + 1,
        }
    }

    pub fn register_dir(&mut self, parent: Ino, name: &str, dir_ref: DirRef) -> Ino {
        let ino = self.ino_for(parent, name);
        match self.nodes.get_mut(&ino) {
            Some(Node::Dir { dir_ref: slot, .. }) => *slot = dir_ref,
            _ => {
                self.nodes.insert(
                    ino,
                    Node::Dir {
                        dir_ref,
                        parent,
                        name: name.to_string(),
                    },
                );
            }
        }
        ino
    }

    pub fn register_file(&mut self, parent: Ino, name: &str) -> Ino {
        let ino = self.ino_for(parent, name);
        self.nodes.entry(ino).or_insert_with(|| Node::File {
            parent,
            name: name.to_string(),
        });
        ino
    }

    fn ino_for(&mut self, parent: Ino, name: &str) -> Ino {
        let key = (parent, cu_format::name::nfc(name).into_owned());
        if let Some(&ino) = self.by_parent_name.get(&key) {
            return ino;
        }
        let ino = self.next_ino;
        self.next_ino += 1;
        self.by_parent_name.insert(key, ino);
        ino
    }

    #[must_use]
    pub fn dir_ref(&self, ino: Ino) -> Option<&DirRef> {
        match self.nodes.get(&ino)? {
            Node::Dir { dir_ref, .. } => Some(dir_ref),
            Node::File { .. } => None,
        }
    }

    #[must_use]
    pub fn file_location(&self, ino: Ino) -> Option<(Ino, &str)> {
        match self.nodes.get(&ino)? {
            Node::File { parent, name } => Some((*parent, name.as_str())),
            Node::Dir { .. } => None,
        }
    }

    #[must_use]
    pub fn is_dir(&self, ino: Ino) -> bool {
        matches!(self.nodes.get(&ino), Some(Node::Dir { .. }))
    }

    #[must_use]
    pub fn parent(&self, ino: Ino) -> Option<Ino> {
        match self.nodes.get(&ino)? {
            Node::Dir { parent, .. } | Node::File { parent, .. } => Some(*parent),
        }
    }

    #[must_use]
    pub fn find(&self, parent: Ino, name: &str) -> Option<Ino> {
        let key = (parent, cu_format::name::nfc(name).into_owned());
        self.by_parent_name.get(&key).copied()
    }

    #[must_use]
    pub fn path_of(&self, ino: Ino) -> Option<String> {
        let mut parts: Vec<&str> = Vec::new();
        let mut cur = ino;
        while cur != ROOT_INO {
            let (Node::Dir { parent, name, .. } | Node::File { parent, name }) =
                self.nodes.get(&cur)?;
            let (parent, name) = (*parent, name.as_str());
            parts.push(name);
            cur = parent;
        }
        parts.reverse();
        Some(parts.join("/"))
    }

    pub fn remove_entry(&mut self, parent: Ino, name: &str) -> Option<Ino> {
        let key = (parent, cu_format::name::nfc(name).into_owned());
        let ino = self.by_parent_name.remove(&key)?;
        self.nodes.remove(&ino);
        Some(ino)
    }

    pub fn rename_entry(
        &mut self,
        old_parent: Ino,
        old_name: &str,
        new_parent: Ino,
        new_name: &str,
    ) -> Option<Ino> {
        let old_key = (old_parent, cu_format::name::nfc(old_name).into_owned());
        let new_key = (new_parent, cu_format::name::nfc(new_name).into_owned());
        let ino = self.by_parent_name.remove(&old_key)?;
        if let Some(replaced) = self.by_parent_name.remove(&new_key) {
            self.nodes.remove(&replaced);
        }
        match self.nodes.get_mut(&ino) {
            Some(Node::Dir { parent, name, .. } | Node::File { parent, name }) => {
                *parent = new_parent;
                *name = new_name.to_string();
            }
            None => {}
        }
        self.by_parent_name.insert(new_key, ino);
        Some(ino)
    }
}
