// bole-81z
//! The immutable pack format (`.pack` + `.idx`).
//!
//! A pack is a self-verifying sequence of per-object frames: a fixed header, one
//! independently-zstd-compressed frame per object (each carrying the object's
//! BLAKE3 id and lengths), and a trailer with a whole-pack digest. Because every
//! frame is self-identifying and independently decodable, the exact same bytes
//! serve as the on-disk pack and the on-wire transfer payload (WS5). The `.idx`
//! is a derived, `mmap`-friendly sorted `ObjectId -> (offset, len)` table.

use crate::error::{Error, Result};
use crate::object::ObjectId;

/// `.pack` header magic.
pub const PACK_MAGIC: &[u8; 8] = b"BOLEPACK";
/// `.pack` trailer / end-of-stream magic.
pub const END_MAGIC: &[u8; 8] = b"BOLEPKND";
/// `.idx` magic.
pub const IDX_MAGIC: &[u8; 8] = b"BOLEIDX\0";
/// Pack format version.
pub const PACK_VERSION: u32 = 1;
/// Index format version.
pub const IDX_VERSION: u32 = 1;
/// Frame record type: a whole object. `0x02` is reserved for a future delta.
pub const RECORD_OBJECT: u8 = 0x01;

const HEADER_LEN: usize = 32;
const TRAILER_LEN: usize = 40;

// bole-oby
/// Maximum accepted uncompressed size of a single packed object (128 MiB).
/// Bounds zstd expansion so a decompression bomb on untrusted wire input cannot
/// exhaust memory before the length/id verification runs.
pub const MAX_OBJECT_LEN: u64 = 128 * 1024 * 1024;
// bole-oby
/// Maximum number of frames accepted in one pack.
pub const MAX_PACK_OBJECTS: u64 = 8_000_000;
// bole-oby
/// Maximum total uncompressed bytes across all frames in one pack (1 GiB), so
/// many small bomb frames cannot amplify past a fixed budget.
pub const MAX_PACK_TOTAL_LEN: u64 = 1024 * 1024 * 1024;
// bole-oby
/// zstd window-log cap (2^27 = 128 MiB), bounding the decoder's internal window
/// regardless of what the compressed frame's header requests.
const ZSTD_WINDOW_LOG_MAX: u32 = 27;

// bole-oby
/// Bounded zstd decode: never materialises more than `max_out + 1` output bytes,
/// so a decompression bomb is caught by allocation limit + the length check that
/// follows. `max_out` is the frame's declared uncompressed length (already
/// verified `<= MAX_OBJECT_LEN`).
fn zstd_decode_bounded(zstd: &[u8], max_out: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut dec = zstd::stream::read::Decoder::new(zstd)
        .map_err(|e| Error::Storage(format!("pack: zstd init: {e}")))?;
    dec.window_log_max(ZSTD_WINDOW_LOG_MAX)
        .map_err(|e| Error::Storage(format!("pack: zstd window: {e}")))?;
    let mut canonical = Vec::new();
    // Read at most max_out + 1 bytes: exactly-right output lands under the cap;
    // a bomb producing more is truncated at cap+1 and rejected by length check.
    dec.take(max_out + 1)
        .read_to_end(&mut canonical)
        .map_err(|e| Error::Storage(format!("pack: zstd decode: {e}")))?;
    Ok(canonical)
}

// LEB128 unsigned varint.
fn write_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let mut byte = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if v == 0 {
            break;
        }
    }
}

fn read_varint(data: &[u8], pos: &mut usize) -> Result<u64> {
    let mut result: u64 = 0;
    let mut shift = 0u32;
    loop {
        let byte = *data.get(*pos).ok_or_else(|| Error::Storage("pack: varint truncated".into()))?;
        *pos += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift >= 64 {
            return Err(Error::Storage("pack: varint overflow".into()));
        }
    }
    Ok(result)
}

// bole-81z
/// One object's location inside a pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackEntry {
    pub id: ObjectId,
    pub offset: u64,
    pub len: u64,
}

// bole-81z
/// Accumulates objects and emits the pack bytes + index entries + digest.
#[derive(Default)]
pub struct PackBuilder {
    objects: Vec<(ObjectId, Vec<u8>)>,
}

impl PackBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an object by its id and canonical (postcard) bytes.
    pub fn add(&mut self, id: ObjectId, canonical: Vec<u8>) {
        self.objects.push((id, canonical));
    }

    /// The number of objects queued.
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Serialises the pack. Returns `(pack_bytes, index_entries, pack_digest)`.
    pub fn finish(self) -> Result<(Vec<u8>, Vec<PackEntry>, [u8; 32])> {
        let mut buf = Vec::new();
        buf.extend_from_slice(PACK_MAGIC);
        buf.extend_from_slice(&PACK_VERSION.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes()); // flags (no dictionary in v1)
        buf.extend_from_slice(&(self.objects.len() as u64).to_le_bytes());
        buf.extend_from_slice(&[0u8; 8]); // reserved

        let mut entries = Vec::with_capacity(self.objects.len());
        for (id, canonical) in &self.objects {
            let offset = buf.len() as u64;
            let zstd = zstd::encode_all(canonical.as_slice(), 3)
                .map_err(|e| Error::Storage(format!("pack: zstd encode: {e}")))?;
            buf.push(RECORD_OBJECT);
            buf.extend_from_slice(id.as_bytes());
            write_varint(&mut buf, canonical.len() as u64);
            write_varint(&mut buf, zstd.len() as u64);
            buf.extend_from_slice(&zstd);
            let len = buf.len() as u64 - offset;
            entries.push(PackEntry { id: *id, offset, len });
        }

        let digest = *blake3::hash(&buf).as_bytes();
        buf.extend_from_slice(END_MAGIC);
        buf.extend_from_slice(&digest);
        Ok((buf, entries, digest))
    }
}

// bole-81z
/// Decodes and verifies a single frame (`record_type`, id, lengths, zstd body),
/// returning `(id, canonical_bytes)`. Verifies `len == uncompressed_len` and
/// `BLAKE3(bytes) == object_id`.
fn decode_frame(frame: &[u8]) -> Result<(ObjectId, Vec<u8>)> {
    let mut pos = 0usize;
    let rt = *frame.get(pos).ok_or_else(|| Error::Storage("pack: empty frame".into()))?;
    pos += 1;
    if rt != RECORD_OBJECT {
        return Err(Error::Storage(format!("pack: unknown record type {rt:#x}")));
    }
    let id_bytes = frame
        .get(pos..pos + 32)
        .ok_or_else(|| Error::Storage("pack: frame id truncated".into()))?;
    let mut idb = [0u8; 32];
    idb.copy_from_slice(id_bytes);
    let id = ObjectId::new(idb);
    pos += 32;
    let ulen = read_varint(frame, &mut pos)?;
    // bole-oby: reject an over-cap declared length before allocating/decoding.
    if ulen > MAX_OBJECT_LEN {
        return Err(Error::Storage(format!(
            "pack: object length {ulen} exceeds cap {MAX_OBJECT_LEN}"
        )));
    }
    let slen = read_varint(frame, &mut pos)? as usize;
    let zstd = frame
        .get(pos..pos + slen)
        .ok_or_else(|| Error::Storage("pack: frame body truncated".into()))?;
    // bole-oby: bounded decode caps output at ulen+1; a bomb is rejected here.
    let canonical = zstd_decode_bounded(zstd, ulen)?;
    if canonical.len() as u64 != ulen {
        return Err(Error::Storage("pack: uncompressed length mismatch".into()));
    }
    if ObjectId::from_content(&canonical) != id {
        return Err(Error::Storage("pack: frame id does not match content".into()));
    }
    Ok((id, canonical))
}

// bole-81z
/// Decodes and verifies a single self-contained frame slice, returning
/// `(id, canonical_bytes)`. Used by the packed backend to read one object.
pub fn decode_frame_public(frame: &[u8]) -> Result<(ObjectId, Vec<u8>)> {
    decode_frame(frame)
}

// bole-81z
/// Random-access read of one object at `(offset, len)` in a pack, verifying it.
pub fn read_frame_at(pack: &[u8], offset: u64, len: u64) -> Result<(ObjectId, Vec<u8>)> {
    let start = offset as usize;
    let end = start
        .checked_add(len as usize)
        .filter(|e| *e <= pack.len())
        .ok_or_else(|| Error::Storage("pack: frame range out of bounds".into()))?;
    decode_frame(&pack[start..end])
}

// bole-81z
/// Fully decodes and verifies a pack: header, every frame (self-verifying), the
/// `object_count`, the trailer `end_magic`, and the whole-pack `pack_digest`.
/// Any truncation or tampering is rejected. This is also the streaming-receive
/// verification (a WS5 receiver reuses [`decode_frame`] per frame).
pub fn decode_pack(pack: &[u8]) -> Result<Vec<(ObjectId, Vec<u8>)>> {
    if pack.len() < HEADER_LEN + TRAILER_LEN {
        return Err(Error::Storage("pack: shorter than header+trailer".into()));
    }
    if &pack[0..8] != PACK_MAGIC {
        return Err(Error::Storage("pack: bad magic".into()));
    }
    let version = u32::from_le_bytes(pack[8..12].try_into().unwrap());
    if version != PACK_VERSION {
        return Err(Error::Storage(format!("pack: unsupported version {version}")));
    }
    let count = u64::from_le_bytes(pack[16..24].try_into().unwrap());
    // bole-oby: reject an absurd object count before the decode loop.
    if count > MAX_PACK_OBJECTS {
        return Err(Error::Storage(format!(
            "pack: object count {count} exceeds cap {MAX_PACK_OBJECTS}"
        )));
    }

    let body_end = pack.len() - TRAILER_LEN;
    let trailer = &pack[body_end..];
    if &trailer[0..8] != END_MAGIC {
        return Err(Error::Storage("pack: bad end magic".into()));
    }
    let body = &pack[..body_end];
    if blake3::hash(body).as_bytes() != &trailer[8..40] {
        return Err(Error::Storage("pack: digest mismatch".into()));
    }

    let mut out = Vec::new();
    let mut total_out: u64 = 0; // bole-oby: running decompressed-bytes budget
    let mut pos = HEADER_LEN;
    while pos < body.len() {
        // Parse frame header to find its total length, then decode+verify it.
        let frame_start = pos;
        let rt = body[pos];
        pos += 1;
        if rt != RECORD_OBJECT {
            return Err(Error::Storage(format!("pack: unknown record type {rt:#x}")));
        }
        pos = pos
            .checked_add(32)
            .filter(|p| *p <= body.len())
            .ok_or_else(|| Error::Storage("pack: frame id truncated".into()))?;
        let _ulen = read_varint(body, &mut pos)?;
        let slen = read_varint(body, &mut pos)? as usize;
        let frame_end = pos
            .checked_add(slen)
            .filter(|e| *e <= body.len())
            .ok_or_else(|| Error::Storage("pack: frame body truncated".into()))?;
        let (id, canonical) = decode_frame(&body[frame_start..frame_end])?;
        // bole-oby: enforce the whole-pack decompressed budget as we go.
        total_out = total_out.saturating_add(canonical.len() as u64);
        if total_out > MAX_PACK_TOTAL_LEN {
            return Err(Error::Storage(format!(
                "pack: total uncompressed size exceeds cap {MAX_PACK_TOTAL_LEN}"
            )));
        }
        out.push((id, canonical));
        pos = frame_end;
    }
    if out.len() as u64 != count {
        return Err(Error::Storage("pack: object count mismatch".into()));
    }
    Ok(out)
}

// bole-81z
/// A sorted `ObjectId -> (offset, len)` index for one pack.
#[derive(Debug, Clone)]
pub struct PackIndex {
    entries: Vec<PackEntry>, // ascending by id
    pack_digest: [u8; 32],
}

impl PackIndex {
    /// Builds an index from a pack's entries (sorted by id) and its digest.
    pub fn build(mut entries: Vec<PackEntry>, pack_digest: [u8; 32]) -> Self {
        entries.sort_by(|a, b| a.id.as_bytes().cmp(b.id.as_bytes()));
        Self { entries, pack_digest }
    }

    pub fn count(&self) -> usize {
        self.entries.len()
    }

    pub fn pack_digest(&self) -> &[u8; 32] {
        &self.pack_digest
    }

    /// All ids in ascending order (ideal for sync set-difference).
    pub fn ids(&self) -> impl Iterator<Item = &ObjectId> {
        self.entries.iter().map(|e| &e.id)
    }

    /// Binary-searches for `id`, returning `(offset, len)` on a hit.
    pub fn lookup(&self, id: &ObjectId) -> Option<(u64, u64)> {
        let first = id.as_bytes()[0];
        let lo = if first == 0 { 0 } else { self.fanout_at(first - 1) };
        let hi = self.fanout_at(first);
        let slice = &self.entries[lo..hi];
        slice
            .binary_search_by(|e| e.id.as_bytes().cmp(id.as_bytes()))
            .ok()
            .map(|i| {
                let e = &slice[i];
                (e.offset, e.len)
            })
    }

    fn fanout_at(&self, byte: u8) -> usize {
        // #entries whose id[0] <= byte.
        self.entries.partition_point(|e| e.id.as_bytes()[0] <= byte)
    }

    /// Serialises the index: magic, version, count, 256-fanout, ids, offsets,
    /// lens, pack_digest, and a trailing idx_digest.
    pub fn encode(&self) -> Vec<u8> {
        let n = self.entries.len();
        let mut buf = Vec::new();
        buf.extend_from_slice(IDX_MAGIC);
        buf.extend_from_slice(&IDX_VERSION.to_le_bytes());
        buf.extend_from_slice(&(n as u32).to_le_bytes());
        // fanout[256]: cumulative count of ids with id[0] <= b.
        for b in 0u16..256 {
            let count = self.entries.partition_point(|e| e.id.as_bytes()[0] <= b as u8);
            buf.extend_from_slice(&(count as u32).to_le_bytes());
        }
        for e in &self.entries {
            buf.extend_from_slice(e.id.as_bytes());
        }
        for e in &self.entries {
            buf.extend_from_slice(&e.offset.to_le_bytes());
        }
        for e in &self.entries {
            buf.extend_from_slice(&e.len.to_le_bytes());
        }
        buf.extend_from_slice(&self.pack_digest);
        let idx_digest = *blake3::hash(&buf).as_bytes();
        buf.extend_from_slice(&idx_digest);
        buf
    }

    /// Parses an encoded index, verifying magic, version, and idx_digest.
    pub fn parse(data: &[u8]) -> Result<PackIndex> {
        let mut pos = 0usize;
        let need = |pos: usize, n: usize, data: &[u8]| -> Result<()> {
            if pos + n > data.len() {
                Err(Error::Storage("idx: truncated".into()))
            } else {
                Ok(())
            }
        };
        need(pos, 16, data)?;
        if &data[0..8] != IDX_MAGIC {
            return Err(Error::Storage("idx: bad magic".into()));
        }
        let version = u32::from_le_bytes(data[8..12].try_into().unwrap());
        if version != IDX_VERSION {
            return Err(Error::Storage(format!("idx: unsupported version {version}")));
        }
        let n = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
        pos = 16;
        // idx_digest is the last 32 bytes; verify it covers everything before.
        if data.len() < 32 {
            return Err(Error::Storage("idx: truncated".into()));
        }
        let (head, idx_digest) = data.split_at(data.len() - 32);
        if blake3::hash(head).as_bytes() != idx_digest {
            return Err(Error::Storage("idx: digest mismatch".into()));
        }
        pos += 256 * 4; // skip fanout
        need(pos, n * 32 + n * 8 + n * 8 + 32, head)?;
        let ids_start = pos;
        let offsets_start = ids_start + n * 32;
        let lens_start = offsets_start + n * 8;
        let digest_start = lens_start + n * 8;

        let mut entries = Vec::with_capacity(n);
        for i in 0..n {
            let mut idb = [0u8; 32];
            idb.copy_from_slice(&head[ids_start + i * 32..ids_start + i * 32 + 32]);
            let offset = u64::from_le_bytes(
                head[offsets_start + i * 8..offsets_start + i * 8 + 8].try_into().unwrap(),
            );
            let len =
                u64::from_le_bytes(head[lens_start + i * 8..lens_start + i * 8 + 8].try_into().unwrap());
            entries.push(PackEntry { id: ObjectId::new(idb), offset, len });
        }
        let mut pack_digest = [0u8; 32];
        pack_digest.copy_from_slice(&head[digest_start..digest_start + 32]);
        Ok(PackIndex { entries, pack_digest })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(n: u8) -> (ObjectId, Vec<u8>) {
        // Canonical bytes are arbitrary here; id is their BLAKE3 (self-verifying).
        let bytes = vec![n; (n as usize) * 3 + 1];
        (ObjectId::from_content(&bytes), bytes)
    }

    #[test]
    fn pack_roundtrip_and_random_access() {
        let objs: Vec<_> = (1u8..20).map(obj).collect();
        let mut b = PackBuilder::new();
        for (id, bytes) in &objs {
            b.add(*id, bytes.clone());
        }
        let (pack, entries, digest) = b.finish().unwrap();
        let idx = PackIndex::build(entries, digest);

        // decode_pack yields every object, verified.
        let decoded = decode_pack(&pack).unwrap();
        assert_eq!(decoded.len(), objs.len());

        // random access via the index.
        for (id, bytes) in &objs {
            let (off, len) = idx.lookup(id).expect("id in index");
            let (rid, rbytes) = read_frame_at(&pack, off, len).unwrap();
            assert_eq!(&rid, id);
            assert_eq!(&rbytes, bytes);
        }
        // a miss returns None.
        assert!(idx.lookup(&ObjectId::from_content(b"absent")).is_none());
        assert_eq!(idx.count(), objs.len());
    }

    #[test]
    fn index_encode_parse_roundtrip() {
        let objs: Vec<_> = (0u8..40).map(obj).collect();
        let mut b = PackBuilder::new();
        for (id, bytes) in &objs {
            b.add(*id, bytes.clone());
        }
        let (_pack, entries, digest) = b.finish().unwrap();
        let idx = PackIndex::build(entries, digest);
        let encoded = idx.encode();
        let parsed = PackIndex::parse(&encoded).unwrap();
        assert_eq!(parsed.count(), idx.count());
        assert_eq!(parsed.pack_digest(), idx.pack_digest());
        for (id, _) in &objs {
            assert_eq!(parsed.lookup(id), idx.lookup(id));
        }
    }

    #[test]
    fn truncated_pack_is_rejected() {
        let mut b = PackBuilder::new();
        let (id, bytes) = obj(5);
        b.add(id, bytes);
        let (pack, _, _) = b.finish().unwrap();
        // Chop the trailer.
        assert!(decode_pack(&pack[..pack.len() - 10]).is_err());
    }

    #[test]
    fn tampered_frame_is_rejected() {
        let mut b = PackBuilder::new();
        for (id, bytes) in (1u8..4).map(obj) {
            b.add(id, bytes);
        }
        let (mut pack, _, _) = b.finish().unwrap();
        // Flip a byte inside the first frame's compressed body (after header).
        let i = HEADER_LEN + 1 + 32 + 2 + 1;
        pack[i] ^= 0xff;
        // Digest covers the body, so the whole-pack check fails.
        assert!(decode_pack(&pack).is_err());
    }

    #[test]
    fn wrong_end_magic_is_rejected() {
        let mut b = PackBuilder::new();
        let (id, bytes) = obj(2);
        b.add(id, bytes);
        let (mut pack, _, _) = b.finish().unwrap();
        let end = pack.len() - TRAILER_LEN;
        pack[end] ^= 0xff; // corrupt end magic
        assert!(decode_pack(&pack).is_err());
    }

    #[test]
    fn index_digest_mismatch_rejected() {
        let mut b = PackBuilder::new();
        let (id, bytes) = obj(3);
        b.add(id, bytes);
        let (_pack, entries, digest) = b.finish().unwrap();
        let mut encoded = PackIndex::build(entries, digest).encode();
        encoded[20] ^= 0xff; // corrupt a fanout byte
        assert!(PackIndex::parse(&encoded).is_err());
    }

    // bole-oby
    /// Hand-crafts a single-frame pack with an attacker-declared `ulen` and a
    /// real zstd body of `payload`, bypassing PackBuilder so the declared length
    /// can lie. Returns the full pack bytes.
    fn crafted_pack(declared_ulen: u64, payload: &[u8]) -> Vec<u8> {
        let zstd = zstd::encode_all(payload, 3).unwrap();
        let id = ObjectId::from_content(payload);
        let mut frame = Vec::new();
        frame.push(RECORD_OBJECT);
        frame.extend_from_slice(id.as_bytes());
        write_varint(&mut frame, declared_ulen);
        write_varint(&mut frame, zstd.len() as u64);
        frame.extend_from_slice(&zstd);

        let mut buf = Vec::new();
        buf.extend_from_slice(PACK_MAGIC);
        buf.extend_from_slice(&PACK_VERSION.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&1u64.to_le_bytes()); // object_count = 1
        buf.extend_from_slice(&[0u8; 8]);
        buf.extend_from_slice(&frame);
        let digest = *blake3::hash(&buf).as_bytes();
        buf.extend_from_slice(END_MAGIC);
        buf.extend_from_slice(&digest);
        buf
    }

    // bole-oby
    #[test]
    fn frame_declaring_oversized_ulen_is_rejected_before_decode() {
        // A frame declaring an uncompressed length above the per-object cap must
        // be refused by the cap check, not decoded. Body is a tiny valid zstd.
        let pack = crafted_pack(MAX_OBJECT_LEN + 1, b"small");
        let err = decode_pack(&pack).unwrap_err();
        assert!(
            format!("{err}").contains("exceeds"),
            "expected an over-cap rejection, got: {err}"
        );
    }

    // bole-oby
    #[test]
    fn frame_with_output_exceeding_declared_ulen_is_rejected() {
        // Declared ulen lies small; the real body decompresses to more. The
        // bounded streaming decoder must stop and reject on the length mismatch.
        let payload = vec![0u8; 1024 * 1024]; // 1 MiB of zeros -> tiny zstd
        let pack = crafted_pack(8, &payload);
        assert!(decode_pack(&pack).is_err());
    }

    // bole-oby
    #[test]
    fn genuine_bomb_capped_by_object_limit() {
        // A real bomb: MAX_OBJECT_LEN+1 compressible bytes. Honestly declared,
        // it must be rejected by the cap without materialising the output.
        let payload = vec![0u8; (MAX_OBJECT_LEN as usize) + 1];
        let pack = crafted_pack(MAX_OBJECT_LEN + 1, &payload);
        let err = decode_pack(&pack).unwrap_err();
        assert!(format!("{err}").contains("exceeds"), "got: {err}");
    }

    #[test]
    fn fanout_boundary_ids() {
        // Force ids with first byte 0x00 and 0xff.
        let e0 = PackEntry { id: ObjectId::new([0u8; 32]), offset: 32, len: 10 };
        let mut hi = [0xffu8; 32];
        hi[31] = 0x01;
        let e1 = PackEntry { id: ObjectId::new(hi), offset: 42, len: 12 };
        let idx = PackIndex::build(vec![e1, e0], [7u8; 32]);
        assert_eq!(idx.lookup(&ObjectId::new([0u8; 32])), Some((32, 10)));
        assert_eq!(idx.lookup(&ObjectId::new(hi)), Some((42, 12)));
        assert!(idx.lookup(&ObjectId::new([0x80u8; 32])).is_none());
    }
}
