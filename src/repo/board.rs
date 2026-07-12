// bole-lkv3
//! Message-board post persistence for a `Repository`.
//!
//! Posts are signed, content-addressed objects pinned per board so they survive
//! GC and can be enumerated. Verification is fail-closed on both write and read.

use crate::board::{verify_post, Post};
use crate::error::{Error, Result};
use crate::object::{Object, ObjectId};
use crate::refs::{Ref, RefName, Tag};
use crate::repo::Repository;

// bole-lkv3
/// Ref prefix under which board posts are pinned:
/// `refs/board/<board>/<post-id>`. Pinning makes posts GC-roots and lets
/// `list_posts` enumerate a board.
pub const BOARD_PREFIX: &str = "refs/board/";

impl Repository {
    // bole-lkv3
    /// Publishes a signed [`Post`]: verifies it (fail-closed), stores it, and
    /// pins it under `refs/board/<board>/<id>` so it survives GC and appears in
    /// [`list_posts`](Repository::list_posts). Returns the post's id.
    pub async fn publish_post(&self, p: &Post) -> Result<ObjectId> {
        if !verify_post(p) {
            return Err(Error::PolicyViolation("board post signature does not verify".into()));
        }
        let id = self.objects.put(&Object::Post(p.clone())).await?;
        let name = RefName::new(format!("{BOARD_PREFIX}{}/{id}", p.board))?;
        let mut tx = self.refs.transaction();
        tx.set(name, Ref::Tag(Tag { target: id, created_at: 0, message: None }));
        tx.commit()?;
        Ok(id)
    }

    // bole-lkv3
    /// Loads the [`Post`] at `id`, verified fail-closed. `None` if absent, not a
    /// post, or unverifiable.
    pub async fn get_post(&self, id: &ObjectId) -> Result<Option<Post>> {
        match self.objects.get(id).await? {
            Some(Object::Post(p)) if verify_post(&p) => Ok(Some(p)),
            _ => Ok(None),
        }
    }

    // bole-lkv3
    /// Every post on `board` (id + post), verified fail-closed.
    pub async fn list_posts(&self, board: &str) -> Result<Vec<(ObjectId, Post)>> {
        let prefix = format!("{BOARD_PREFIX}{board}/");
        let mut out = Vec::new();
        for name in self.refs.list(&prefix)? {
            if let Some(tag) = self.refs.get_tag(&name)? {
                if let Some(p) = self.get_post(&tag.target).await? {
                    out.push((tag.target, p));
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use crate::board::BoardSigner;
    use crate::Repository;

    #[tokio::test]
    async fn publish_list_and_survives_gc() {
        let repo = Repository::memory();
        let signer = BoardSigner::from_seed([10u8; 32]);
        let a = repo.publish_post(&signer.sign_post("general", "first", None, 1)).await.unwrap();
        let _b = repo.publish_post(&signer.sign_post("general", "second", Some(a), 2)).await.unwrap();
        // A post on a different board is not listed under "general".
        repo.publish_post(&signer.sign_post("random", "elsewhere", None, 3)).await.unwrap();

        let mut posts = repo.list_posts("general").await.unwrap();
        posts.sort_by_key(|(_, p)| p.created_at);
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0].1.body, "first");
        assert_eq!(posts[1].1.parent, Some(a), "reply threads to its parent");
        assert_eq!(repo.list_posts("random").await.unwrap().len(), 1);

        // Pinned posts survive GC.
        repo.gc(&[], 0, 1_000_000).await.unwrap();
        assert_eq!(repo.list_posts("general").await.unwrap().len(), 2, "posts survive GC");
    }

    #[tokio::test]
    async fn publish_rejects_unsigned() {
        let repo = Repository::memory();
        let signer = BoardSigner::from_seed([11u8; 32]);
        let mut bad = signer.sign_post("b", "body", None, 0);
        bad.body = "tampered".into();
        assert!(repo.publish_post(&bad).await.is_err(), "tampered post refused");
    }

    #[tokio::test]
    async fn get_absent_and_wrong_type_is_none() {
        let repo = Repository::memory();
        let missing = crate::ObjectId::from_content(b"nope");
        assert!(repo.get_post(&missing).await.unwrap().is_none());
        let blob = repo.objects.put_blob(bytes::Bytes::from("x")).await.unwrap();
        assert!(repo.get_post(&blob).await.unwrap().is_none());
    }
}
