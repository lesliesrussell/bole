// bole-l0i
use crate::object::ObjectId;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceView {
    pub files: BTreeMap<String, ObjectId>,
    pub env: BTreeMap<String, String>,
}
