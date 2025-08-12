use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Simple JSON-pointer-like path
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ValuePath {
    segs: Vec<PathSeg>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PathSeg {
    Key(String),
    Index(usize),
}

impl ValuePath {
    pub fn new() -> Self { Self { segs: Vec::new() } }
    pub fn push_key(mut self, k: impl ToString) -> Self { self.segs.push(PathSeg::Key(k.to_string())); self }
    pub fn push_idx(mut self, i: usize) -> Self { self.segs.push(PathSeg::Index(i)); self }
    pub fn segs(&self) -> &[PathSeg] { &self.segs }
}