// bole-dq0
use crate::object::ObjectId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub root: ObjectId,
    pub parents: Vec<ObjectId>,
    pub author: String,
    pub created_at: u64,
    pub message: String,
}
