// bole-18p
use async_trait::async_trait;

use crate::collab::CollabObject;
use crate::error::Result;

/// The interface a node (and, later, a relay) exposes to serve its
/// **public-labeled** collaboration objects. v1 implements only the
/// sovereign-node side (`impl for Repository`); relays are a future impl of the
/// same trait, so discovery client code needs no change when they land.
#[async_trait]
pub trait PublicObjectSource {
    /// Every public collaboration object this source is willing to serve. MUST
    /// return only objects pinned under the public prefix — never scoped objects.
    async fn public_objects(&self) -> Result<Vec<CollabObject>>;
}
