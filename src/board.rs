// bole-lkv3
//! Message-board posts — the object layer of the discussion board.
//!
//! A [`Post`] is a signed, content-addressed message on a named board. Like a
//! [`Profile`](crate::Profile) it is metadata only — it grants nothing and
//! never overrides the lattice/ACLs. Threading is by `parent`: a reply points
//! at the post it answers (`None` for a top-level post). This slice defines the
//! object, its signing, and fail-closed verification; later slices add the CLI
//! and a read API.

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::collab::Key;
use crate::object::ObjectId;

// bole-lkv3
/// Domain-separation tag for board-post signatures.
const BOARD_POST_DOMAIN: &[u8] = b"bole-board-post-v1\0";

// bole-lkv3
/// A signed message on a board. `board` is the board name; `parent` is the post
/// this one replies to (`None` = a top-level post). Canonical author is
/// `author` (its key verifies `sig`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Post {
    pub author: Key,
    pub board: String,
    pub body: String,
    pub parent: Option<ObjectId>,
    pub created_at: u64,
    /// Ed25519 signature (64 bytes) over the domain-separated unsigned fields.
    pub sig: Vec<u8>,
}

// bole-lkv3
#[derive(Serialize)]
struct PostMsg<'a> {
    author: &'a Key,
    board: &'a str,
    body: &'a str,
    parent: &'a Option<ObjectId>,
    created_at: u64,
}

// bole-lkv3
fn post_message(p: &Post) -> Vec<u8> {
    let mut m = BOARD_POST_DOMAIN.to_vec();
    let body = postcard::to_allocvec(&PostMsg {
        author: &p.author,
        board: &p.board,
        body: &p.body,
        parent: &p.parent,
        created_at: p.created_at,
    })
    .expect("postcard serialization is infallible for owned data");
    m.extend_from_slice(&body);
    m
}

// bole-lkv3
/// Signs board [`Post`]s under a held Ed25519 key. Mirrors
/// [`CollabSigner`](crate::CollabSigner).
pub struct BoardSigner {
    signing: SigningKey,
}

impl BoardSigner {
    /// Builds a signer from a 32-byte seed.
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self { signing: SigningKey::from_bytes(&seed) }
    }

    /// The public key that authors — and verifies — this signer's posts.
    pub fn public_key(&self) -> Key {
        self.signing.verifying_key().to_bytes()
    }

    /// Signs a post on `board`. `parent` is the post being replied to, or
    /// `None` for a new top-level post.
    pub fn sign_post(
        &self,
        board: impl Into<String>,
        body: impl Into<String>,
        parent: Option<ObjectId>,
        created_at: u64,
    ) -> Post {
        let mut p = Post {
            author: self.public_key(),
            board: board.into(),
            body: body.into(),
            parent,
            created_at,
            sig: Vec::new(),
        };
        p.sig = self.signing.sign(&post_message(&p)).to_bytes().to_vec();
        p
    }
}

// bole-lkv3
/// Verifies a post's signature against its embedded author key. Fail-closed: a
/// malformed key or signature returns `false`.
pub fn verify_post(p: &Post) -> bool {
    let vk = match VerifyingKey::from_bytes(&p.author) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let bytes: [u8; 64] = match p.sig.as_slice().try_into() {
        Ok(b) => b,
        Err(_) => return false,
    };
    vk.verify(&post_message(p), &ed25519_dalek::Signature::from_bytes(&bytes)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_and_tamper() {
        let signer = BoardSigner::from_seed([1u8; 32]);
        let p = signer.sign_post("general", "hello", None, 5);
        assert_eq!(p.author, signer.public_key());
        assert_eq!(p.board, "general");
        assert!(p.parent.is_none());
        assert!(verify_post(&p));

        // Every field is signed.
        let mut p1 = signer.sign_post("general", "hi", None, 1);
        p1.body = "evil".into();
        assert!(!verify_post(&p1), "tampered body");
        let mut p2 = signer.sign_post("general", "hi", None, 1);
        p2.board = "other".into();
        assert!(!verify_post(&p2), "tampered board");
        let mut p3 = signer.sign_post("general", "hi", None, 1);
        p3.parent = Some(ObjectId::from_content(b"x"));
        assert!(!verify_post(&p3), "tampered parent");
        let mut p4 = signer.sign_post("general", "hi", None, 1);
        p4.created_at = 999;
        assert!(!verify_post(&p4), "tampered created_at");
        let mut p5 = signer.sign_post("general", "hi", None, 1);
        p5.author = BoardSigner::from_seed([2u8; 32]).public_key();
        assert!(!verify_post(&p5), "swapped author");
    }

    #[test]
    fn malformed_signature_is_false_not_panic() {
        let signer = BoardSigner::from_seed([3u8; 32]);
        let mut p = signer.sign_post("b", "body", None, 0);
        p.sig = vec![0u8; 3];
        assert!(!verify_post(&p));
    }
}
