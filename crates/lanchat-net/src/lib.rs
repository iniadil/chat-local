//! ChatLocal networking core.
//!
//! A node listens on TCP, announces itself with mDNS, connects to every peer it
//! learns about (discovered or configured by hand) and exchanges
//! length-prefixed JSON frames with optional binary payloads. A node with
//! `relay` enabled additionally forwards traffic between its peers, which lets
//! machines that cannot see each other directly (different subnets, mDNS
//! blocked) still talk through one well-known address.

pub mod discovery;
pub mod node;
pub mod store;
pub mod types;
pub mod wire;

pub use node::{Node, NodeConfig};
pub use types::*;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("frame of {0} bytes exceeds the limit")]
    FrameTooLarge(usize),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("peer {0} is not reachable")]
    Unreachable(String),
    #[error("attachment too large ({0} bytes)")]
    AttachmentTooLarge(u64),
    #[error("discovery error: {0}")]
    Discovery(String),
    #[error("{0}")]
    Other(String),
}

/// Local, non-loopback IPv4 addresses, for showing the user what to type on
/// other machines.
pub fn local_ipv4_addresses() -> Vec<String> {
    match local_ip_address::list_afinet_netifas() {
        Ok(list) => {
            let mut v: Vec<String> = list
                .into_iter()
                .filter_map(|(_, ip)| match ip {
                    std::net::IpAddr::V4(ip)
                        if !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified() =>
                    {
                        Some(ip.to_string())
                    }
                    _ => None,
                })
                .collect();
            v.sort();
            v.dedup();
            v
        }
        Err(_) => Vec::new(),
    }
}
