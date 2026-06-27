// bole-prn
use crate::object::ObjectId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TimelinePolicy {
    FastForwardOnly,
    Append,
    Unrestricted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    pub head: ObjectId,
    pub policy: TimelinePolicy,
    pub created_at: u64,
}
