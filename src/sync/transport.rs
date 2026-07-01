// bole-6qy
//! The transport abstraction: [`Conn`], a duplex control-message channel the
//! protocol ([`super::session`]) is written against, plus [`InProcessConn`], an
//! in-memory implementation and the backbone of protocol testing. Concrete
//! network transports (TCP/HTTP/SSH) implement the same trait (bole-vih).

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::error::{Error, Result};
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
