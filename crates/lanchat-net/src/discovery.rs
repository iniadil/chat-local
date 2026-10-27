//! mDNS / DNS-SD announcement and browsing (via the `mdns-sd` crate).
//!
//! The daemon runs on its own OS threads; we bridge its events into a Tokio
//! channel so the node can consume them asynchronously.

use crate::types::{PeerId, SERVICE_TYPE};
use crate::Error;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use tokio::sync::mpsc;

/// A peer announced on the LAN.
#[derive(Clone, Debug)]
pub struct Discovered {
    pub id: PeerId,
    pub name: String,
    pub addrs: Vec<SocketAddr>,
}

pub struct Discovery {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Discovery {
    /// Announce ourselves and start browsing. Discovered peers (never
    /// ourselves) are delivered on the returned channel.
    pub fn start(
        id: &PeerId,
        name: &str,
        port: u16,
    ) -> Result<(Self, mpsc::Receiver<Discovered>), Error> {
        let daemon = ServiceDaemon::new().map_err(|e| Error::Discovery(e.to_string()))?;

        let short: String = id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(12)
            .collect();
        let host = format!("chatlocal-{short}.local.");
        let mut props = HashMap::new();
        props.insert("id".to_string(), id.clone());
        props.insert("name".to_string(), name.to_string());
        let info = ServiceInfo::new(SERVICE_TYPE, &short, &host, (), port, props)
            .map_err(|e| Error::Discovery(e.to_string()))?
            .enable_addr_auto();
        let fullname = info.get_fullname().to_string();
        daemon
            .register(info)
            .map_err(|e| Error::Discovery(e.to_string()))?;

        let browser = daemon
            .browse(SERVICE_TYPE)
            .map_err(|e| Error::Discovery(e.to_string()))?;

        let (tx, rx) = mpsc::channel(64);
        let my_id = id.clone();
        std::thread::Builder::new()
            .name("mdns-browse".into())
            .spawn(move || loop {
                match browser.recv_timeout(Duration::from_secs(1)) {
                    Ok(ServiceEvent::ServiceResolved(svc)) => {
                        let Some(pid) = svc.get_property_val_str("id") else {
                            continue;
                        };
                        if pid == my_id {
                            continue;
                        }
                        let port = svc.get_port();
                        let mut addrs: Vec<SocketAddr> = svc
                            .get_addresses()
                            .iter()
                            .map(|ip| SocketAddr::new(ip.to_ip_addr(), port))
                            .filter(|a| !a.ip().is_loopback())
                            .collect();
                        // Prefer IPv4: it is what people see in their router and
                        // what the manual-peer field expects.
                        addrs.sort_by_key(|a| matches!(a.ip(), IpAddr::V6(_)));
                        if addrs.is_empty() {
                            continue;
                        }
                        let d = Discovered {
                            id: pid.to_string(),
                            name: svc.get_property_val_str("name").unwrap_or("").to_string(),
                            addrs,
                        };
                        if tx.blocking_send(d).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(mdns_sd::RecvTimeoutError::Timeout) => {
                        if tx.is_closed() {
                            break;
                        }
                    }
                    Err(mdns_sd::RecvTimeoutError::Disconnected) => break,
                }
            })
            .map_err(|e| Error::Discovery(e.to_string()))?;

        Ok((Self { daemon, fullname }, rx))
    }

    pub fn stop(&self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.stop_browse(SERVICE_TYPE);
        let _ = self.daemon.shutdown();
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        self.stop();
    }
}
