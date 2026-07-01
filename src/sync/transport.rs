// bole-6qy
//! The transport abstraction: [`Conn`], a duplex control-message channel the
//! protocol ([`super::session`]) is written against, plus [`InProcessConn`], an
//! in-memory implementation and the backbone of protocol testing. Concrete
//! network transports (TCP/HTTP/SSH) implement the same trait (bole-vih).

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use crate::acl::Accessor;
use crate::error::{Error, Result};
use crate::repo::Repository;
use crate::sync::wire::{self, Message};

// bole-6qy
/// A duplex, ordered, reliable channel for protocol control messages. The
/// session code knows only this trait, never a socket.
#[async_trait]
pub trait Conn: Send {
    /// Sends one control message.
    async fn send(&mut self, msg: &Message) -> Result<()>;
    /// Receives the next control message (awaits until one arrives).
    async fn recv(&mut self) -> Result<Message>;
}

// bole-6qy
/// An in-memory [`Conn`] backed by a pair of unbounded channels. Bodies are
/// encoded with the real [`wire`] codec so the protocol exercises the same
/// encoding a network transport would.
pub struct InProcessConn {
    tx: mpsc::UnboundedSender<Vec<u8>>,
    rx: mpsc::UnboundedReceiver<Vec<u8>>,
}

impl InProcessConn {
    /// Returns two connected ends: a message sent on one arrives on the other.
    pub fn pair() -> (InProcessConn, InProcessConn) {
        let (tx1, rx1) = mpsc::unbounded_channel();
        let (tx2, rx2) = mpsc::unbounded_channel();
        (InProcessConn { tx: tx1, rx: rx2 }, InProcessConn { tx: tx2, rx: rx1 })
    }
}

#[async_trait]
impl Conn for InProcessConn {
    async fn send(&mut self, msg: &Message) -> Result<()> {
        let bytes = wire::encode_message(msg)?;
        self.tx.send(bytes).map_err(|_| Error::Storage("connection closed".into()))
    }

    async fn recv(&mut self) -> Result<Message> {
        let bytes = self.rx.recv().await.ok_or_else(|| Error::Storage("connection closed".into()))?;
        wire::decode_message(&bytes)
    }
}

// bole-vih
/// A [`Conn`] over a persistent TCP connection: control messages are
/// length-prefixed frames ([`wire::frame`]) written to / read from the socket.
/// The `SyncSession` message loop runs unchanged over it.
pub struct TcpConn {
    stream: TcpStream,
    rbuf: Vec<u8>,
    rpos: usize,
}

impl TcpConn {
    /// Wraps an established stream (client side: `TcpStream::connect`).
    pub fn new(stream: TcpStream) -> Self {
        Self { stream, rbuf: Vec::new(), rpos: 0 }
    }

    /// Connects to `addr` (e.g. `"127.0.0.1:9418"`).
    pub async fn connect(addr: &str) -> Result<Self> {
        let stream = TcpStream::connect(addr).await.map_err(Error::Io)?;
        Ok(Self::new(stream))
    }
}

#[async_trait]
impl Conn for TcpConn {
    async fn send(&mut self, msg: &Message) -> Result<()> {
        let framed = wire::frame(&wire::encode_message(msg)?);
        self.stream.write_all(&framed).await.map_err(Error::Io)
    }

    async fn recv(&mut self) -> Result<Message> {
        loop {
            let mut pos = self.rpos;
            // Copy the framed body out so the immutable borrow of rbuf ends
            // before we compact it.
            if let Some(body) = wire::deframe(&self.rbuf, &mut pos)?.map(|b| b.to_vec()) {
                self.rpos = pos;
                if self.rpos > 1 << 16 {
                    self.rbuf.drain(..self.rpos);
                    self.rpos = 0;
                }
                return wire::decode_message(&body);
            }
            let mut chunk = [0u8; 8192];
            let n = self.stream.read(&mut chunk).await.map_err(Error::Io)?;
            if n == 0 {
                return Err(Error::Storage("connection closed".into()));
            }
            self.rbuf.extend_from_slice(&chunk[..n]);
        }
    }
}

// bole-vih
/// Accepts one connection on `listener` and serves a single sync session for it,
/// authorizing with `accessor`. Returns after the session completes. A caller
/// runs this in a loop (or per-connection task) for a long-lived server.
pub async fn serve_tcp_once(
    listener: &TcpListener,
    repo: &Repository,
    accessor: &Accessor,
) -> Result<()> {
    let (stream, _peer) = listener.accept().await.map_err(Error::Io)?;
    let mut conn = TcpConn::new(stream);
    crate::sync::session::serve(&mut conn, repo, accessor).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{EntryKind, ObjectId, Snapshot, TreeEntry};
    use crate::refs::{RefName, TimelinePolicy};
    use crate::repo::Repository;
    use crate::sync::session::client_fetch;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    async fn seed(repo: &Repository, name: &str) -> ObjectId {
        let blob = repo.objects.put_blob(bytes::Bytes::from_static(b"tcp")).await.unwrap();
        let mut e = BTreeMap::new();
        e.insert("f".to_string(), TreeEntry { id: blob, kind: EntryKind::Blob });
        let tree = repo.objects.put_tree(e).await.unwrap();
        let snap = repo
            .objects
            .put_snapshot(Snapshot { root: tree, parents: vec![], author: "t".into(), created_at: 0, message: "m".into() })
            .await
            .unwrap();
        repo.refs
            .create_timeline(RefName::new(name).unwrap(), snap, TimelinePolicy::Unrestricted, 0, "persistent".into(), None)
            .unwrap();
        snap
    }

    #[tokio::test]
    async fn fetch_over_tcp_loopback() {
        let server = Arc::new(Repository::memory());
        let head = seed(&server, "main").await;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let srv = server.clone();
        let handle = tokio::spawn(async move {
            serve_tcp_once(&listener, &srv, &Accessor::privileged()).await
        });

        let mut conn = TcpConn::connect(&addr).await.unwrap();
        let client = Repository::memory();
        let tracked = client_fetch(&mut conn, &client, "origin").await.unwrap();
        handle.await.unwrap().unwrap();

        assert_eq!(tracked.len(), 1);
        assert!(client.objects.get(&head).await.unwrap().is_some(), "object arrived over TCP");
        let tref = RefName::new("refs/remotes/origin/main").unwrap();
        assert_eq!(client.refs.get_tag(&tref).unwrap().unwrap().target, head);
    }
}
