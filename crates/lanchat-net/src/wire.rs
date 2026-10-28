//! Length-prefixed framing over TCP.
//!
//! ```text
//! +-----------+-----------+---------------+------------------+
//! | u32 total | u32 jsonN | json[jsonN]   | payload[...]     |
//! +-----------+-----------+---------------+------------------+
//! ```
//!
//! `total` counts everything after itself. `json` is a [`Frame`]. `payload`
//! is only non-empty for [`Frame::Chunk`], where it holds up to
//! [`CHUNK_BYTES`] of attachment data. An attachment travels as a
//! [`Frame::Message`] announcing it, then its chunks, then [`Frame::End`]
//! (or [`Frame::Abort`] if the sender gave up), all tagged with the
//! message id. Chunks of different transfers, and control frames, may be
//! interleaved on one connection.

use crate::types::{Message, PeerId, CHUNK_BYTES, MAX_FRAME_BYTES, PROTOCOL_VERSION};
use crate::Error;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RosterEntry {
    pub id: PeerId,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Frame {
    /// First frame in each direction.
    Hello {
        id: PeerId,
        name: String,
        port: u16,
        version: u32,
        relay: bool,
    },
    /// A text message, or the announcement of an attachment whose bytes
    /// follow as chunks.
    Message {
        message: Message,
    },
    /// One piece of the attachment of message `id`; the bytes are the payload.
    Chunk {
        id: String,
    },
    /// Every chunk of `id` has been sent.
    End {
        id: String,
    },
    /// The sender could not finish `id`; discard what was received.
    Abort {
        id: String,
    },
    /// Sent by relays: the peers currently connected to it.
    Roster {
        peers: Vec<RosterEntry>,
    },
    Ping,
    Pong,
}

/// What a peer said about itself in its [`Frame::Hello`], once checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    pub id: PeerId,
    pub name: String,
    pub port: u16,
    pub relay: bool,
}

impl Frame {
    /// Interpret the first frame of a connection: it must be a hello from a
    /// peer that speaks our protocol version.
    pub fn into_hello(self) -> Result<Hello, Error> {
        let Frame::Hello {
            id,
            name,
            port,
            version,
            relay,
        } = self
        else {
            return Err(Error::Protocol("expected hello".into()));
        };
        if version != PROTOCOL_VERSION {
            return Err(Error::Protocol(format!(
                "peer speaks protocol v{version}, we speak v{PROTOCOL_VERSION}"
            )));
        }
        Ok(Hello {
            id,
            name,
            port,
            relay,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Packet {
    pub frame: Frame,
    pub payload: Vec<u8>,
}

impl Packet {
    pub fn frame(frame: Frame) -> Self {
        Self {
            frame,
            payload: Vec::new(),
        }
    }

    pub fn chunk(id: &str, payload: Vec<u8>) -> Self {
        debug_assert!(payload.len() <= CHUNK_BYTES);
        Self {
            frame: Frame::Chunk { id: id.to_string() },
            payload,
        }
    }
}

pub async fn write_packet<W: AsyncWrite + Unpin>(w: &mut W, pkt: &Packet) -> Result<(), Error> {
    let json = serde_json::to_vec(&pkt.frame)?;
    let total = 4 + json.len() + pkt.payload.len();
    if total > MAX_FRAME_BYTES {
        return Err(Error::FrameTooLarge(total));
    }
    let mut head = Vec::with_capacity(8 + json.len());
    head.extend_from_slice(&(total as u32).to_be_bytes());
    head.extend_from_slice(&(json.len() as u32).to_be_bytes());
    head.extend_from_slice(&json);
    w.write_all(&head).await?;
    if !pkt.payload.is_empty() {
        w.write_all(&pkt.payload).await?;
    }
    w.flush().await?;
    Ok(())
}

pub async fn read_packet<R: AsyncRead + Unpin>(r: &mut R) -> Result<Packet, Error> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await?;
    let total = u32::from_be_bytes(len) as usize;
    if total < 4 {
        return Err(Error::Protocol("frame too short".into()));
    }
    if total > MAX_FRAME_BYTES {
        return Err(Error::FrameTooLarge(total));
    }
    r.read_exact(&mut len).await?;
    let json_len = u32::from_be_bytes(len) as usize;
    if json_len > total - 4 {
        return Err(Error::Protocol("header length exceeds frame".into()));
    }
    let mut json = vec![0u8; json_len];
    r.read_exact(&mut json).await?;
    let frame: Frame = serde_json::from_slice(&json)?;
    let payload_len = total - 4 - json_len;
    if payload_len > 0 && !matches!(frame, Frame::Chunk { .. }) {
        return Err(Error::Protocol("payload on a non-chunk frame".into()));
    }
    let mut payload = vec![0u8; payload_len];
    if payload_len > 0 {
        r.read_exact(&mut payload).await?;
    }
    Ok(Packet { frame, payload })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn roundtrip_with_payload() {
        let pkt = Packet::chunk("m1", vec![1, 2, 3, 4, 5]);
        let mut buf = Vec::new();
        write_packet(&mut buf, &pkt).await.unwrap();
        let mut cur = std::io::Cursor::new(buf);
        let back = read_packet(&mut cur).await.unwrap();
        assert_eq!(back.frame, pkt.frame);
        assert_eq!(back.payload, pkt.payload);
    }

    #[tokio::test]
    async fn rejects_oversized_header() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(u32::MAX).to_be_bytes());
        let mut cur = std::io::Cursor::new(buf);
        assert!(matches!(
            read_packet(&mut cur).await,
            Err(Error::FrameTooLarge(_))
        ));
    }

    #[tokio::test]
    async fn rejects_payload_on_control_frame() {
        let pkt = Packet {
            frame: Frame::Ping,
            payload: vec![0; 10],
        };
        let mut buf = Vec::new();
        write_packet(&mut buf, &pkt).await.unwrap();
        let mut cur = std::io::Cursor::new(buf);
        assert!(matches!(
            read_packet(&mut cur).await,
            Err(Error::Protocol(_))
        ));
    }
}
