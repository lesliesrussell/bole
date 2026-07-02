// bole-6qy
//! The sync wire protocol: the [`Message`] control-frame enum and its codec.
//!
//! Messages are postcard-encoded (sharing bole's on-disk encoding for `ObjectId`
//! / `RefName` / `Ref`). A message travels as a length-prefixed frame so a byte
//! stream (a future TCP/HTTP transport) can delimit frames; the in-memory
//! transport sends the encoded body directly. Pack bytes ride as a `Pack`
//! variant in v1 (streaming bodies are a transport optimization, deferred).

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::object::ObjectId;
use crate::refs::RefName;

/// The only protocol version in v1.
pub const PROTO_VERSION: u16 = 1;

// bole-6qy
/// Capability bits negotiated in the handshake (all reserved in v1; sides
/// operate at the intersection, unknown bits ignored).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapSet(pub u32);

impl CapSet {
    pub const EMPTY: CapSet = CapSet(0);
    pub fn intersect(self, other: CapSet) -> CapSet {
        CapSet(self.0 & other.0)
    }
}

// bole-6qy
/// What a client wants to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Intent {
    Fetch,
    Push,
    Clone,
}

// bole-6qy
/// An advertised ref (name, target head/commit, and whether it is a timeline).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefAdvert {
    pub name: RefName,
    pub target: ObjectId,
    pub is_timeline: bool,
}

// bole-6qy
/// A requested compare-and-swap ref update (push).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefUpdateOp {
    pub name: RefName,
    pub expected_old: Option<ObjectId>,
    pub new_head: ObjectId,
}

// bole-6qy
/// The outcome of applying one pushed ref.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefApplyStatus {
    Ok,
    NonFastForward { server_head: ObjectId },
    Denied(String),
}

// bole-6qy
/// Per-ref push result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefStatusEntry {
    pub name: RefName,
    pub status: RefApplyStatus,
}

// bole-6qy
/// One control frame of the sync protocol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    /// client → server: version range, capabilities, intent.
    Hello { proto_min: u16, proto_max: u16, caps: CapSet, intent: Intent },
    /// server → client: chosen version + capabilities + advertised refs.
    Welcome { proto: u16, caps: CapSet, refs: Vec<RefAdvert> },
    /// negotiation: the sender's want (ref targets) and have (object id set).
    HaveWant { want: Vec<ObjectId>, have: Vec<ObjectId> },
    /// a WS4 pack carrying the missing object closure.
    Pack(Vec<u8>),
    /// push: requested CAS ref ops.
    RefUpdate(Vec<RefUpdateOp>),
    /// push result / fetch completion status.
    RefStatus(Vec<RefStatusEntry>),
    /// end of a phase / session.
    Done,
    /// a typed failure (version, auth, policy, corrupt frame, …).
    Error(String),
}

// bole-6qy
/// Encodes a message to its postcard body bytes.
pub fn encode_message(m: &Message) -> Result<Vec<u8>> {
    postcard::to_allocvec(m).map_err(|e| Error::Codec(e.to_string()))
}

// bole-6qy
/// Decodes a message from its postcard body bytes.
pub fn decode_message(bytes: &[u8]) -> Result<Message> {
    postcard::from_bytes(bytes).map_err(|e| Error::Codec(e.to_string()))
}

// bole-6qy
/// Frames a body for a byte stream: a 4-byte little-endian length prefix then the
/// body. Used by stream transports (TCP/HTTP, deferred); the in-memory transport
/// sends bodies directly.
pub fn frame(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    out
}

// bole-oby
/// Maximum accepted length-prefixed frame body on a stream transport (256 MiB).
/// A hostile peer's length prefix above this is rejected before the body is
/// buffered, bounding transport memory. It must be at least as large as a
/// legitimately transferable pack (see [`crate::store::pack::MAX_PACK_TOTAL_LEN`]
/// compressed) plus control-message overhead.
pub const MAX_FRAME_LEN: usize = 256 * 1024 * 1024;

// bole-6qy
/// Reads one length-prefixed frame body from `buf` starting at `*pos`, advancing
/// `*pos` past it. Returns `None` if the buffer does not yet hold a full frame.
pub fn deframe<'a>(buf: &'a [u8], pos: &mut usize) -> Result<Option<&'a [u8]>> {
    if buf.len() < *pos + 4 {
        return Ok(None);
    }
    let len = u32::from_le_bytes(buf[*pos..*pos + 4].try_into().unwrap()) as usize;
    // bole-oby: reject an oversized frame from the length prefix alone, before
    // the body is buffered, so a hostile prefix cannot drive unbounded reads.
    if len > MAX_FRAME_LEN {
        return Err(Error::Storage(format!(
            "frame length {len} exceeds cap {MAX_FRAME_LEN}"
        )));
    }
    if buf.len() < *pos + 4 + len {
        return Ok(None);
    }
    let body = &buf[*pos + 4..*pos + 4 + len];
    *pos += 4 + len;
    Ok(Some(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    // bole-oby
    #[test]
    fn deframe_rejects_oversized_length_prefix() {
        // A 4-byte prefix above MAX_FRAME_LEN must error immediately, even with
        // no body bytes present — proving the body is never buffered.
        let mut buf = ((MAX_FRAME_LEN as u32) + 1).to_le_bytes().to_vec();
        buf.extend_from_slice(b"x"); // nowhere near the declared length
        let mut pos = 0;
        assert!(deframe(&buf, &mut pos).is_err());
    }

    // bole-oby
    #[test]
    fn deframe_accepts_within_cap() {
        let framed = frame(b"hello");
        let mut pos = 0;
        assert_eq!(deframe(&framed, &mut pos).unwrap(), Some(&b"hello"[..]));
    }

    #[test]
    fn message_encode_decode_roundtrip() {
        let m = Message::Hello {
            proto_min: 1,
            proto_max: 1,
            caps: CapSet::EMPTY,
            intent: Intent::Fetch,
        };
        let bytes = encode_message(&m).unwrap();
        assert_eq!(decode_message(&bytes).unwrap(), m);

        let w = Message::Welcome {
            proto: 1,
            caps: CapSet(0),
            refs: vec![RefAdvert {
                name: RefName::new("main").unwrap(),
                target: ObjectId::from_content(b"x"),
                is_timeline: true,
            }],
        };
        assert_eq!(decode_message(&encode_message(&w).unwrap()).unwrap(), w);
    }

    #[test]
    fn frame_deframe_stream() {
        let a = encode_message(&Message::Done).unwrap();
        let b = encode_message(&Message::Error("boom".into())).unwrap();
        let mut stream = Vec::new();
        stream.extend_from_slice(&frame(&a));
        stream.extend_from_slice(&frame(&b));

        let mut pos = 0;
        let f1 = deframe(&stream, &mut pos).unwrap().unwrap();
        assert_eq!(decode_message(f1).unwrap(), Message::Done);
        let f2 = deframe(&stream, &mut pos).unwrap().unwrap();
        assert_eq!(decode_message(f2).unwrap(), Message::Error("boom".into()));
        // No more full frames.
        assert!(deframe(&stream, &mut pos).unwrap().is_none());
    }

    #[test]
    fn deframe_partial_returns_none() {
        let a = frame(&encode_message(&Message::Done).unwrap());
        let mut pos = 0;
        // Only the length prefix, no body yet.
        assert!(deframe(&a[..4], &mut pos).unwrap().is_none());
    }
}
