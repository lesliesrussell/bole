// bole-vih
//! A minimal HTTP/1.1 transport (`POST /bole/v1/{fetch,push}`), one request per
//! verb — the stateless mapping the spec's §7.2/§9 recommends. It does not use a
//! web framework; it speaks just enough HTTP/1.1 (request line + headers +
//! Content-Length body) over a tokio `TcpStream` to be driven by curl or fronted
//! by a reverse proxy. TLS and chunked encoding are deferred.
//!
//! Unlike the `SyncSession` message loop, each verb is a single round trip:
//! - **fetch** — the client sends its `have`; the server replies with its
//!   advertised refs plus a pack of the missing closure of ALL readable heads
//!   (no `Welcome` round trip is needed because the client wants everything it
//!   may read).
//! - **push** — the client sends the pack + CAS ops (expected-old from its
//!   remote-tracking refs); the server replies with per-ref status.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::acl::Accessor;
use crate::error::{Error, Result};
use crate::object::ObjectId;
use crate::refs::{Ref, RefName, Tag};
use crate::repo::Repository;
use crate::store::pack::decode_pack;
use crate::sync::negotiate;
use crate::sync::session::{advertise, apply_push_ops, build_pack};
use crate::sync::wire::{RefAdvert, RefApplyStatus, RefStatusEntry, RefUpdateOp};

const FETCH_PATH: &str = "/bole/v1/fetch";
const PUSH_PATH: &str = "/bole/v1/push";

#[derive(Serialize, Deserialize)]
struct FetchReq {
    have: Vec<ObjectId>,
}
#[derive(Serialize, Deserialize)]
struct FetchResp {
    refs: Vec<RefAdvert>,
    pack: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
struct PushReq {
    pack: Vec<u8>,
    ops: Vec<RefUpdateOp>,
}
#[derive(Serialize, Deserialize)]
struct PushResp {
    status: Vec<RefStatusEntry>,
}

// bole-vih
/// Reads one HTTP message (headers + Content-Length body) from `stream`,
/// returning `(start_line, body)`.
async fn read_http(stream: &mut TcpStream) -> Result<(String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let header_end = loop {
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
            break pos;
        }
        let n = stream.read(&mut tmp).await.map_err(Error::Io)?;
        if n == 0 {
            return Err(Error::Storage("http: connection closed before headers".into()));
        }
        buf.extend_from_slice(&tmp[..n]);
    };
    let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let start_line = headers.lines().next().unwrap_or("").to_string();
    let content_length = headers
        .lines()
        .find_map(|l| {
            let low = l.to_ascii_lowercase();
            low.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))
        })
        .unwrap_or(0);
    let mut body = buf[header_end + 4..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut tmp).await.map_err(Error::Io)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(content_length);
    Ok((start_line, body))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

async fn post(stream: &mut TcpStream, path: &str, body: &[u8]) -> Result<Vec<u8>> {
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: bole\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.map_err(Error::Io)?;
    stream.write_all(body).await.map_err(Error::Io)?;
    let (status_line, resp) = read_http(stream).await?;
    if !status_line.contains(" 200") {
        return Err(Error::Storage(format!("http: server returned '{status_line}'")));
    }
    Ok(resp)
}

async fn respond(stream: &mut TcpStream, body: &[u8]) -> Result<()> {
    let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    stream.write_all(head.as_bytes()).await.map_err(Error::Io)?;
    stream.write_all(body).await.map_err(Error::Io)?;
    Ok(())
}

fn tracking_ref(remote_name: &str, name: &RefName) -> Result<RefName> {
    RefName::new(format!("refs/remotes/{remote_name}/{}", name.as_str()))
        .map_err(|e| Error::Storage(format!("bad tracking ref name: {e}")))
}

// bole-vih
/// Client: `POST /bole/v1/fetch` — pull the peer's readable closure into `local`
/// and set `refs/remotes/<remote_name>/*`.
pub async fn http_fetch(
    addr: &str,
    local: &Repository,
    remote_name: &str,
) -> Result<Vec<(RefName, ObjectId)>> {
    let mut stream = TcpStream::connect(addr).await.map_err(Error::Io)?;
    let have = local.objects.list().await?;
    let body = postcard::to_allocvec(&FetchReq { have }).map_err(|e| Error::Codec(e.to_string()))?;
    let resp = post(&mut stream, FETCH_PATH, &body).await?;
    let resp: FetchResp = postcard::from_bytes(&resp).map_err(|e| Error::Codec(e.to_string()))?;

    for (_id, canonical) in decode_pack(&resp.pack)? {
        local.objects.put_raw(&canonical).await?;
    }
    let mut tx = local.refs.transaction();
    let mut tracked = Vec::new();
    for r in &resp.refs {
        let tref = tracking_ref(remote_name, &r.name)?;
        tx.set(tref.clone(), Ref::Tag(Tag { target: r.target, created_at: 0, message: None }));
        tracked.push((tref, r.target));
    }
    tx.commit()?;
    Ok(tracked)
}

// bole-vih
/// Client: `POST /bole/v1/push` — CAS the peer's heads against this repo's
/// remote-tracking refs. Returns per-ref status; advances tracking on accepts.
pub async fn http_push(
    addr: &str,
    local: &Repository,
    remote_name: &str,
    timelines: &[RefName],
) -> Result<Vec<RefStatusEntry>> {
    let mut ops = Vec::new();
    let mut wants = Vec::new();
    let mut have = Vec::new();
    for name in timelines {
        let tl = match local.refs.get_timeline(name)? {
            Some(t) => t,
            None => continue,
        };
        let tracking = tracking_ref(remote_name, name)?;
        let expected_old = local.refs.get_tag(&tracking)?.map(|t| t.target);
        if let Some(old) = expected_old {
            have.push(old);
        }
        wants.push(tl.head);
        ops.push(RefUpdateOp { name: name.clone(), expected_old, new_head: tl.head });
    }
    let have: HashSet<ObjectId> = have.into_iter().collect();
    let missing = negotiate::missing_closure(local, &wants, &have).await?;
    let pack = build_pack(local, &missing).await?;

    let mut stream = TcpStream::connect(addr).await.map_err(Error::Io)?;
    let body = postcard::to_allocvec(&PushReq { pack, ops: ops.clone() })
        .map_err(|e| Error::Codec(e.to_string()))?;
    let resp = post(&mut stream, PUSH_PATH, &body).await?;
    let resp: PushResp = postcard::from_bytes(&resp).map_err(|e| Error::Codec(e.to_string()))?;

    let mut tx = local.refs.transaction();
    for entry in &resp.status {
        if entry.status == RefApplyStatus::Ok {
            if let Some(op) = ops.iter().find(|o| o.name == entry.name) {
                tx.set(
                    tracking_ref(remote_name, &entry.name)?,
                    Ref::Tag(Tag { target: op.new_head, created_at: 0, message: None }),
                );
            }
        }
    }
    tx.commit()?;
    Ok(resp.status)
}

// bole-vih
/// Server: accept one HTTP request on `listener`, route it, and respond.
/// `accessor` gates advertised reads and authorizes pushes.
pub async fn serve_http_once(
    listener: &TcpListener,
    repo: &Repository,
    accessor: &Accessor,
) -> Result<()> {
    let (mut stream, _peer) = listener.accept().await.map_err(Error::Io)?;
    let (start_line, body) = read_http(&mut stream).await?;

    if start_line.contains(FETCH_PATH) {
        let req: FetchReq = postcard::from_bytes(&body).map_err(|e| Error::Codec(e.to_string()))?;
        let refs = advertise(repo, accessor)?;
        let want: Vec<ObjectId> = refs.iter().map(|r| r.target).collect();
        let have: HashSet<ObjectId> = req.have.into_iter().collect();
        let missing = negotiate::missing_closure(repo, &want, &have).await?;
        let pack = build_pack(repo, &missing).await?;
        let out = postcard::to_allocvec(&FetchResp { refs, pack }).map_err(|e| Error::Codec(e.to_string()))?;
        respond(&mut stream, &out).await
    } else if start_line.contains(PUSH_PATH) {
        let req: PushReq = postcard::from_bytes(&body).map_err(|e| Error::Codec(e.to_string()))?;
        for (_id, canonical) in decode_pack(&req.pack)? {
            repo.objects.put_raw(&canonical).await?;
        }
        let status = apply_push_ops(repo, accessor, &req.ops).await?;
        let out = postcard::to_allocvec(&PushResp { status }).map_err(|e| Error::Codec(e.to_string()))?;
        respond(&mut stream, &out).await
    } else {
        let head = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        stream.write_all(head.as_bytes()).await.map_err(Error::Io)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acl::clearance::{Capability, Clearance, ClearanceScope, ClearanceSet};
    use crate::acl::lattice::{Label, LabelLattice};
    use crate::acl::rules::LabelRuleSet;
    use crate::object::{EntryKind, Snapshot, TreeEntry};
    use crate::refs::TimelinePolicy;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    fn writer() -> Accessor {
        let clr = ClearanceSet {
            clearances: vec![Clearance {
                ceiling: Label::protected(),
                cap: Capability::WRITE,
                scope: Some(ClearanceScope::Timeline("**".into())),
            }],
            confined: false,
        };
        Accessor::from_parts(Arc::new(LabelLattice::two_point()), Arc::new(LabelRuleSet::default()), clr)
    }

    async fn commit(repo: &Repository, parent: Option<ObjectId>, payload: &[u8]) -> ObjectId {
        let blob = repo.objects.put_blob(bytes::Bytes::copy_from_slice(payload)).await.unwrap();
        let mut e = BTreeMap::new();
        e.insert("f".to_string(), TreeEntry { id: blob, kind: EntryKind::Blob });
        let tree = repo.objects.put_tree(e).await.unwrap();
        repo.objects
            .put_snapshot(Snapshot { root: tree, parents: parent.into_iter().collect(), author: "t".into(), created_at: 0, message: "m".into() })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn fetch_then_push_over_http() {
        let server = Arc::new(Repository::memory());
        let base = commit(&server, None, b"base").await;
        let name = RefName::new("main").unwrap();
        server.refs.create_timeline(name.clone(), base, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();

        // --- fetch over HTTP ---
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let srv = server.clone();
        let h = tokio::spawn(async move { serve_http_once(&listener, &srv, &Accessor::privileged()).await });

        let client = Repository::memory();
        let tracked = http_fetch(&addr, &client, "origin").await.unwrap();
        h.await.unwrap().unwrap();
        assert_eq!(tracked.len(), 1);
        assert!(client.objects.get(&base).await.unwrap().is_some());

        // --- advance locally + push over HTTP ---
        client.refs.create_timeline(name.clone(), base, TimelinePolicy::Unrestricted, 0, "persistent".into(), None).unwrap();
        let next = commit(&client, Some(base), b"next").await;
        client.refs.advance_head(&name, next).unwrap();

        let listener2 = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr2 = listener2.local_addr().unwrap().to_string();
        let srv2 = server.clone();
        let h2 = tokio::spawn(async move { serve_http_once(&listener2, &srv2, &writer()).await });

        let status = http_push(&addr2, &client, "origin", std::slice::from_ref(&name)).await.unwrap();
        h2.await.unwrap().unwrap();

        assert_eq!(status.len(), 1);
        assert_eq!(status[0].status, RefApplyStatus::Ok);
        assert_eq!(server.refs.get_timeline(&name).unwrap().unwrap().head, next);
    }
}
