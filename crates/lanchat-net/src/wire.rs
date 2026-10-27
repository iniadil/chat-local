//! Length-prefixed framing over TCP.
//!
//! ```text
//! +-----------+-----------+---------------+------------------+
//! | u32 total | u32 jsonN | json[jsonN]   | payload[...]     |
//! +-----------+-----------+---------------+------------------+
//! ```
//!
//! `total` counts everything after itself. `json` is a [`Frame`]. `payload`
//! is only non-empty for [`Frame::Message`] carrying an attachment, in which
//! case it holds the raw attachment bytes.

use crate::types::{Message, PeerId, MAX_FRAME_BYTES};
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
    Message {
        message: Message,
    },
    /// Sent by relays: the peers currently connected to it.
    Roster {
        peers: Vec<RosterEntry>,
    },
    Ping,
    Pong,
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
        let pkt = Packet {
            frame: Frame::Hello {
                id: "a".into(),
                name: "Alice".into(),
                port: 1,
                version: 1,
                relay: false,
            },
            payload: vec![1, 2, 3, 4, 5],
        };
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
}
