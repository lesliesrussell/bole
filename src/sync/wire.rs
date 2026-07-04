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
    // bole-iz5c
    /// True iff `self` contains every bit of the non-empty `other`.
    pub fn contains(self, other: CapSet) -> bool {
        other.0 != 0 && (self.0 & other.0) == other.0
    }
}

// bole-iz5c
/// Server-side term search (WS8f-b). A relay advertises this in `Welcome.caps`;
/// a client requests it in `Hello.caps`.
pub const CAP_SEARCH: CapSet = CapSet(1 << 0);

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

// bole-nbug
/// Serde helper for `Option<[u8; 64]>`: postcard/serde's blanket array impls only
/// go to 32; this module serialises the 64-byte relay signature as a fixed-length
/// tuple of bytes so the wire representation is compact and round-trips cleanly.
mod opt_sig64 {
    use serde::{Deserializer, Serializer};
    use serde::de::{SeqAccess, Visitor};
    use std::fmt;

    struct Arr64Visitor;
    impl<'de> Visitor<'de> for Arr64Visitor {
        type Value = [u8; 64];
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "a 64-byte sequence")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<[u8; 64], A::Error> {
            let mut arr = [0u8; 64];
            for (i, b) in arr.iter_mut().enumerate() {
                *b = seq.next_element()?.ok_or_else(|| {
                    serde::de::Error::invalid_length(i, &"64 bytes")
                })?;
            }
            Ok(arr)
        }
    }

    struct SerArr64<'a>(&'a [u8; 64]);
    impl serde::Serialize for SerArr64<'_> {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeTuple;
            let mut t = s.serialize_tuple(64)?;
            for b in self.0.iter() {
                t.serialize_element(b)?;
            }
            t.end()
        }
    }

    pub fn serialize<S: Serializer>(v: &Option<[u8; 64]>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(arr) => s.serialize_some(&SerArr64(arr)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<[u8; 64]>, D::Error> {
        struct OptVisitor;
        impl<'de> Visitor<'de> for OptVisitor {
            type Value = Option<[u8; 64]>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "an optional 64-byte sequence")
            }
            fn visit_none<E: serde::de::Error>(self) -> Result<Option<[u8; 64]>, E> {
                Ok(None)
            }
            fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Option<[u8; 64]>, D2::Error> {
                d.deserialize_tuple(64, Arr64Visitor).map(Some)
            }
        }
        d.deserialize_option(OptVisitor)
    }
}

// bole-6qy
/// One control frame of the sync protocol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    // bole-nbug
    /// client → server: version range, capabilities, intent. `client_nonce` is
    /// `Some` only on a relay-auth query; `None` (all other flows) requests no
    /// relay-auth and is byte-compatible behaviour with pre-WS8f-a callers.
    Hello { proto_min: u16, proto_max: u16, caps: CapSet, intent: Intent, client_nonce: Option<[u8; 32]> },
    /// server → client: chosen version + capabilities + advertised refs.
    /// `relay_sig` is `Some` only when a relay with a signer answers a
    /// `client_nonce`; `None` otherwise.
    Welcome { proto: u16, caps: CapSet, refs: Vec<RefAdvert>, #[serde(with = "opt_sig64")] relay_sig: Option<[u8; 64]> },
    /// negotiation: the sender's want (ref targets) and have (object id set).
    HaveWant { want: Vec<ObjectId>, have: Vec<ObjectId> },
    /// a WS4 pack carrying the missing object closure.
    Pack(Vec<u8>),
    /// push: requested CAS ref ops.
    RefUpdate(Vec<RefUpdateOp>),
    /// push result / fetch completion status.
    RefStatus(Vec<RefStatusEntry>),
    // bole-iz5c
    /// client → relay (after Welcome, in place of HaveWant): server-side term
    /// search bounded by `max_hops`. Answered with a `Pack` of matching profiles
    /// + the directed reverse-reachability edge ball, then `Done`.
    Search { term: String, max_hops: u8 },
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
            client_nonce: None,
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
            relay_sig: None,
        };
        assert_eq!(decode_message(&encode_message(&w).unwrap()).unwrap(), w);

        // bole-nbug
        let m = Message::Hello {
            proto_min: 1, proto_max: 1, caps: CapSet::EMPTY, intent: Intent::Fetch,
            client_nonce: Some([7u8; 32]),
        };
        assert_eq!(decode_message(&encode_message(&m).unwrap()).unwrap(), m);
        let w = Message::Welcome {
            proto: 1, caps: CapSet::EMPTY, refs: vec![], relay_sig: Some([9u8; 64]),
        };
        assert_eq!(decode_message(&encode_message(&w).unwrap()).unwrap(), w);

        // bole-iz5c
        let s = Message::Search { term: "pat".into(), max_hops: 4 };
        assert_eq!(decode_message(&encode_message(&s).unwrap()).unwrap(), s);
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
