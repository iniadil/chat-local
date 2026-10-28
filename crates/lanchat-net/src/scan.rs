//! Active LAN scan: find ChatLocal peers without mDNS.
//!
//! Every ChatLocal node writes a [`Frame::Hello`] the moment a TCP connection
//! is accepted, before reading anything. A probe therefore only has to
//! connect, read that first frame and close. Because the probe never sends
//! its own hello, the probed node never registers a peer, so scanning is
//! invisible in its sidebar (it only logs a failed handshake at debug level).
//!
//! The scan walks the `/24` around every local IPv4 address, which matches
//! how home, office and classroom networks are almost always configured.
//! Larger subnets still work for the hosts that share the first three octets
//! with this machine; the rest can be added by address.

use crate::types::{FoundPeer, FoundVia, PeerId};
use crate::wire::read_packet;
use crate::Error;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::BufReader;
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tokio::time::timeout;

/// How long a probe waits for the TCP handshake. Hosts on a LAN answer (or
/// refuse) within a few milliseconds; the rest of the budget covers Wi-Fi
/// power saving.
const PROBE_CONNECT_TIMEOUT: Duration = Duration::from_millis(700);
/// How long a probe waits for the hello once connected.
const PROBE_HELLO_TIMEOUT: Duration = Duration::from_millis(1500);
/// Probes in flight at once. 254 hosts per subnet at this width finish in
/// a couple of seconds even when most addresses are silent.
const PROBE_CONCURRENCY: usize = 96;

/// Ask one address who is there. `Ok(None)` means something answered with a
/// valid hello but it was this very node (its own address was probed).
pub async fn probe(addr: SocketAddr, my_id: &PeerId) -> Result<Option<FoundPeer>, Error> {
    let stream = timeout(PROBE_CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| Error::Other(format!("timed out connecting to {addr}")))??;
    let mut rd = BufReader::with_capacity(4096, stream);
    let pkt = timeout(PROBE_HELLO_TIMEOUT, read_packet(&mut rd))
        .await
        .map_err(|_| Error::Protocol("no hello".into()))??;
    let hello = pkt.frame.into_hello()?;
    if &hello.id == my_id {
        return Ok(None);
    }
    Ok(Some(FoundPeer {
        id: hello.id,
        name: hello.name,
        addr: SocketAddr::new(addr.ip(), hello.port).to_string(),
        relay: hello.relay,
        via: FoundVia::Scan,
        connected: false,
    }))
}

/// The `/24` networks around the given local addresses, deduplicated.
pub fn subnets_of(local: &[Ipv4Addr]) -> Vec<[u8; 3]> {
    let mut v: Vec<[u8; 3]> = local
        .iter()
        .filter(|ip| !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified())
        .map(|ip| {
            let o = ip.octets();
            [o[0], o[1], o[2]]
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Probe every host of every subnet on every port. One entry per peer id
/// (the first address that answered wins), in no particular order.
pub async fn scan(subnets: &[[u8; 3]], ports: &[u16], my_id: &PeerId) -> Vec<FoundPeer> {
    let mut ports: Vec<u16> = ports.iter().copied().filter(|p| *p != 0).collect();
    ports.sort();
    ports.dedup();

    let sem = Arc::new(Semaphore::new(PROBE_CONCURRENCY));
    let mut tasks = tokio::task::JoinSet::new();
    for net in subnets {
        for host in 1..=254u8 {
            let ip = IpAddr::V4(Ipv4Addr::new(net[0], net[1], net[2], host));
            for port in &ports {
                let addr = SocketAddr::new(ip, *port);
                let sem = sem.clone();
                let my_id = my_id.clone();
                tasks.spawn(async move {
                    let _permit = sem.acquire_owned().await.ok()?;
                    match probe(addr, &my_id).await {
                        Ok(found) => found,
                        Err(e) => {
                            tracing::trace!("probe {addr}: {e}");
                            None
                        }
                    }
                });
            }
        }
    }

    let mut found: Vec<FoundPeer> = Vec::new();
    while let Some(res) = tasks.join_next().await {
        if let Ok(Some(p)) = res {
            if !found.iter().any(|f| f.id == p.id) {
                found.push(p);
            }
        }
    }
    found
}
