//! Shared data types: identity, settings, messages, peers and events.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Stable identifier of a node (UUID v4, generated once per installation).
pub type PeerId = String;

/// Bumped whenever the wire format changes incompatibly.
pub const PROTOCOL_VERSION: u32 = 2;
/// mDNS / DNS-SD service type announced on the LAN.
pub const SERVICE_TYPE: &str = "_chatlocal._tcp.local.";
/// Default TCP port a node listens on.
pub const DEFAULT_PORT: u16 = 47800;
/// Size of one attachment chunk on the wire. Attachments are streamed, so
/// this is also roughly how much of a transfer sits in memory at a time.
pub const CHUNK_BYTES: usize = 256 * 1024;
/// Hard cap for a single frame (JSON header + one chunk).
pub const MAX_FRAME_BYTES: usize = CHUNK_BYTES + 64 * 1024;
/// Longest text one message may carry. Keeps a text frame well inside
/// [`MAX_FRAME_BYTES`] so it can never be refused at the wire.
pub const MAX_TEXT_BYTES: usize = 64 * 1024;
/// Cap for a single attachment. Bytes stream from disk to disk, so this only
/// guards against mistakes, not memory.
pub const MAX_ATTACHMENT_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Identity {
    pub id: PeerId,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    /// Name shown to other people.
    pub display_name: String,
    /// TCP port this node listens on.
    pub port: u16,
    /// Announce and look for peers with mDNS.
    pub discovery: bool,
    /// Act as a relay (hub): forward every message to all other connected peers
    /// and publish the list of connected peers to them.
    pub relay: bool,
    /// Extra peers (or a relay) to connect to, as `host:port` or `host`.
    pub manual_peers: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            display_name: String::new(),
            port: DEFAULT_PORT,
            discovery: true,
            relay: false,
            manual_peers: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    Image,
    Audio,
    File,
}

impl AttachmentKind {
    pub fn from_mime(mime: &str) -> Self {
        if mime.starts_with("image/") {
            Self::Image
        } else if mime.starts_with("audio/") {
            Self::Audio
        } else {
            Self::File
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Attachment {
    pub kind: AttachmentKind,
    pub file_name: String,
    pub mime: String,
    pub size: u64,
    /// Hex SHA-256 of the bytes; verified on receipt.
    pub sha256: String,
    /// Length of a voice note, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u32>,
    /// Where the bytes live on *this* machine. Never sent over the wire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub from: PeerId,
    pub from_name: String,
    /// `None` means "everyone" (the shared room).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<PeerId>,
    /// Unix time in milliseconds, as set by the sender.
    pub sent_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<Attachment>,
}

impl Message {
    /// Copy of the message with local-only fields stripped.
    pub fn for_wire(&self) -> Message {
        let mut m = self.clone();
        if let Some(a) = m.attachment.as_mut() {
            a.path = None;
        }
        m
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerInfo {
    pub id: PeerId,
    pub name: String,
    /// Remote address of the direct connection, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addr: Option<String>,
    /// The peer announced itself as a relay.
    pub relay: bool,
    /// We hold a direct TCP connection to this peer.
    pub direct: bool,
    /// When not direct: the relay through which the peer is reachable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<PeerId>,
}

/// How a peer was found before any connection existed.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FoundVia {
    /// Announced itself with mDNS/Bonjour.
    Mdns,
    /// Answered a probe during an active subnet scan.
    Scan,
}

/// A ChatLocal installation seen on the network, whether or not we are
/// connected to it yet. What the "Add computer" dialog lists.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FoundPeer {
    pub id: PeerId,
    pub name: String,
    /// `ip:port` to connect to.
    pub addr: String,
    pub relay: bool,
    pub via: FoundVia,
    /// We already hold a connection to this peer.
    pub connected: bool,
}

/// Something the UI should know about.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Ready {
        port: u16,
        addresses: Vec<String>,
    },
    PeerOnline {
        peer: PeerInfo,
    },
    PeerOffline {
        id: PeerId,
    },
    Message {
        message: Message,
    },
    /// Progress of an attachment transfer, emitted every megabyte or so and
    /// once more when it finishes. `done == total` means complete.
    Transfer {
        message_id: String,
        /// The peer the bytes are going to or coming from.
        peer: PeerId,
        direction: TransferDirection,
        done: u64,
        total: u64,
    },
    /// An attachment transfer that will not complete.
    TransferFailed {
        message_id: String,
        peer: PeerId,
        direction: TransferDirection,
        reason: String,
    },
    Error {
        message: String,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransferDirection {
    Sending,
    Receiving,
}
