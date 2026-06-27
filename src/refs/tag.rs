// bole-prn
use crate::object::ObjectId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tag {
    pub target: ObjectId,
    pub created_at: u64,
    pub message: Option<String>,
}
