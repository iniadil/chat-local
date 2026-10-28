//! The node: listener, outgoing connections, discovery glue, relaying and the
//! public send API.

use crate::discovery::{Discovered, Discovery, DiscoveryEvent};
use crate::store::{copy_hashed, hash_file, Store};
use crate::types::*;
use crate::wire::{read_packet, write_packet, Frame, Packet, RosterEntry};
use crate::Error;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const READ_TIMEOUT: Duration = Duration::from_secs(90);
const PING_INTERVAL: Duration = Duration::from_secs(25);
const RECONNECT_INTERVAL: Duration = Duration::from_secs(5);
const SEEN_CAPACITY: usize = 8192;
/// Control frames (text, hello, pings, transfer announcements) per connection.
const OUTBOX_CAPACITY: usize = 256;
/// Attachment chunks queued per connection. Small on purpose: this, times
/// [`CHUNK_BYTES`], is the memory one connection may hold for transfers.
const DATA_CAPACITY: usize = 8;
/// How long a relay waits for a slow downstream peer to take a chunk before
/// giving up on forwarding that transfer to it.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(30);
/// Transfers one connection may have in flight toward us at once.
const MAX_INCOMING_PER_CONN: usize = 16;
/// An incoming transfer that has not seen a chunk for this long is dropped,
/// so a lost `End`/`Abort` cannot pin a `.part` file and a slot forever.
const INCOMING_IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// Emit a progress event every this many bytes.
const PROGRESS_STEP: u64 = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct NodeConfig {
    pub identity: Identity,
    pub settings: Settings,
    pub data_dir: PathBuf,
}

struct Conn {
    serial: u64,
    info: PeerInfo,
    initiated_by_me: bool,
    /// Control frames: written ahead of anything waiting in `data_tx`.
    tx: mpsc::Sender<Arc<Packet>>,
    /// Attachment chunks; bounded so a transfer backs up at the sender
    /// instead of in memory.
    data_tx: mpsc::Sender<Arc<Packet>>,
    cancel: CancellationToken,
}

/// A connection that accepted a transfer announcement and will take its
/// chunks.
#[derive(Clone)]
struct Target {
    id: PeerId,
    tx: mpsc::Sender<Arc<Packet>>,
    data_tx: mpsc::Sender<Arc<Packet>>,
}

/// An attachment arriving on one connection.
struct Incoming {
    message: Message,
    total: u64,
    received: u64,
    hasher: Sha256,
    /// Open `.part` file, when this node is a recipient.
    file: Option<(File, PathBuf)>,
    /// Connections this relay forwards the chunks to.
    forward: Vec<Target>,
    last_progress: u64,
    /// When the announcement or the latest chunk arrived.
    last_activity: Instant,
}

#[derive(Default)]
struct Inner {
    conns: HashMap<PeerId, Conn>,
    /// Last roster received from each relay we are connected to.
    rosters: HashMap<PeerId, Vec<RosterEntry>>,
    /// Peers reachable only through a relay, derived from `rosters`.
    via_relay: HashMap<PeerId, PeerInfo>,
    /// Peers learned from discovery, by peer id.
    known: HashMap<PeerId, Discovered>,
    /// Addresses with a connection attempt in flight.
    connecting: HashSet<SocketAddr>,
    /// Addresses that turned out to be this very node (someone typed our own
    /// address, or discovery echoed our own announcement).
    self_addrs: HashSet<SocketAddr>,
}

struct Seen {
    order: VecDeque<String>,
    set: HashSet<String>,
}

impl Seen {
    fn insert(&mut self, id: &str) -> bool {
        if self.set.contains(id) {
            return false;
        }
        self.set.insert(id.to_string());
        self.order.push_back(id.to_string());
        if self.order.len() > SEEN_CAPACITY {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
        true
    }
}

pub struct Node {
    pub identity: Identity,
    /// Settings the node was started with. `manual_peers` here is only the
    /// initial list; see [`Node::set_manual_peers`].
    pub settings: Settings,
    /// Addresses the reconnect loop keeps dialing. Changeable at runtime so
    /// remembering a peer does not need a restart.
    manual_peers: Mutex<Vec<String>>,
    port: u16,
    store: Mutex<Store>,
    inner: Mutex<Inner>,
    seen: Mutex<Seen>,
    events: mpsc::Sender<Event>,
    shutdown: CancellationToken,
    next_serial: AtomicU64,
    discovery: Mutex<Option<Discovery>>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

impl Node {
    /// Bind, announce, and start connecting. Returns the node and the stream
    /// of events the UI should render.
    pub async fn start(config: NodeConfig) -> Result<(Arc<Node>, mpsc::Receiver<Event>), Error> {
        let store = Store::open(&config.data_dir)?;
        let listener = TcpListener::bind(("0.0.0.0", config.settings.port)).await?;
        let port = listener.local_addr()?.port();
        let (events_tx, events_rx) = mpsc::channel(256);

        let node = Arc::new(Node {
            identity: Identity {
                id: config.identity.id.clone(),
                name: if config.settings.display_name.trim().is_empty() {
                    config.identity.name.clone()
                } else {
                    config.settings.display_name.trim().to_string()
                },
            },
            settings: config.settings.clone(),
            manual_peers: Mutex::new(config.settings.manual_peers.clone()),
            port,
            store: Mutex::new(store),
            inner: Mutex::new(Inner::default()),
            seen: Mutex::new(Seen {
                order: VecDeque::new(),
                set: HashSet::new(),
            }),
            events: events_tx,
            shutdown: CancellationToken::new(),
            next_serial: AtomicU64::new(1),
            discovery: Mutex::new(None),
        });

        // Accept loop.
        {
            let node = node.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = node.shutdown.cancelled() => break,
                        accepted = listener.accept() => match accepted {
                            Ok((stream, addr)) => {
                                tracing::debug!("inbound connection from {addr}");
                                let n = node.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = n.run_connection(stream, false, None).await {
                                        tracing::debug!("inbound {addr} ended: {e}");
                                    }
                                });
                            }
                            Err(e) => {
                                tracing::warn!("accept failed: {e}");
                                tokio::time::sleep(Duration::from_millis(200)).await;
                            }
                        }
                    }
                }
            });
        }

        // Discovery.
        if config.settings.discovery {
            match Discovery::start(&node.identity.id, &node.identity.name, port) {
                Ok((d, mut rx)) => {
                    *node.discovery.lock().unwrap() = Some(d);
                    let n = node.clone();
                    tokio::spawn(async move {
                        loop {
                            tokio::select! {
                                _ = n.shutdown.cancelled() => break,
                                found = rx.recv() => match found {
                                    Some(DiscoveryEvent::Found(d)) => n.on_discovered(d),
                                    Some(DiscoveryEvent::Lost(id)) => n.on_lost(&id),
                                    None => break,
                                }
                            }
                        }
                    });
                }
                Err(e) => {
                    tracing::warn!("mDNS unavailable: {e}");
                    node.emit(Event::Error {
                        message: format!("Automatic discovery is unavailable: {e}"),
                    })
                    .await;
                }
            }
        }

        // Reconnect loop for manual peers and known addresses.
        {
            let node = node.clone();
            tokio::spawn(async move {
                loop {
                    node.connect_pending().await;
                    tokio::select! {
                        _ = node.shutdown.cancelled() => break,
                        _ = tokio::time::sleep(RECONNECT_INTERVAL) => {}
                    }
                }
            });
        }

        node.emit(Event::Ready {
            port,
            addresses: crate::local_ipv4_addresses(),
        })
        .await;
        Ok((node, events_rx))
    }

    /// Stop everything: listener, discovery, every connection.
    pub fn shutdown(&self) {
        self.shutdown.cancel();
        if let Some(d) = self.discovery.lock().unwrap().take() {
            d.stop();
        }
        let mut inner = self.inner.lock().unwrap();
        for (_, c) in inner.conns.drain() {
            c.cancel.cancel();
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn peers(&self) -> Vec<PeerInfo> {
        let inner = self.inner.lock().unwrap();
        let mut v: Vec<PeerInfo> = inner.conns.values().map(|c| c.info.clone()).collect();
        v.extend(inner.via_relay.values().cloned());
        v.sort_by_key(|p| p.name.to_lowercase());
        v
    }

    pub fn messages(&self) -> Vec<Message> {
        self.store.lock().unwrap().messages().to_vec()
    }

    pub fn find_message(&self, id: &str) -> Option<Message> {
        self.store.lock().unwrap().find(id).cloned()
    }

    pub fn files_dir(&self) -> PathBuf {
        self.store.lock().unwrap().files_dir().to_path_buf()
    }

    pub fn clear_history(&self) -> Result<(), Error> {
        self.store.lock().unwrap().clear()
    }

    async fn emit(&self, ev: Event) {
        let _ = self.events.send(ev).await;
    }

    // ----- sending -------------------------------------------------------

    pub async fn send_text(&self, to: Option<PeerId>, text: String) -> Result<Message, Error> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err(Error::Other("empty message".into()));
        }
        if text.len() > MAX_TEXT_BYTES {
            return Err(Error::Other(format!(
                "message is longer than {} KB",
                MAX_TEXT_BYTES / 1024
            )));
        }
        let msg = self.new_message(to, Some(text), None);
        self.route(&msg, None)?;
        self.store.lock().unwrap().append(msg.clone())?;
        Ok(msg)
    }

    /// Send a file from disk, streaming it from where it is. The original
    /// file stays in place and the message refers to it.
    pub async fn send_file(
        self: &Arc<Self>,
        to: Option<PeerId>,
        path: &Path,
    ) -> Result<Message, Error> {
        let meta = tokio::fs::metadata(path).await?;
        if !meta.is_file() {
            return Err(Error::Other("not a file".into()));
        }
        if meta.len() > MAX_ATTACHMENT_BYTES {
            return Err(Error::AttachmentTooLarge(meta.len()));
        }
        let (size, sha256) = hash_file(path).await?;
        let file_name = file_name_of(path);
        let mime = mime_guess::from_path(path)
            .first_or_octet_stream()
            .essence_str()
            .to_string();
        let attachment = Attachment {
            kind: AttachmentKind::from_mime(&mime),
            file_name,
            mime,
            size,
            sha256,
            duration_ms: None,
            path: Some(path.to_path_buf()),
        };
        let msg = self.new_message(to, None, Some(attachment));
        self.send_attachment(msg).await
    }

    /// Send a file from disk, first copying it into the files directory so
    /// the message keeps working if the original moves. `mime` overrides
    /// detection from the extension.
    pub async fn import_file(
        self: &Arc<Self>,
        to: Option<PeerId>,
        path: &Path,
        mime: Option<String>,
    ) -> Result<Message, Error> {
        let meta = tokio::fs::metadata(path).await?;
        if !meta.is_file() {
            return Err(Error::Other("not a file".into()));
        }
        if meta.len() > MAX_ATTACHMENT_BYTES {
            return Err(Error::AttachmentTooLarge(meta.len()));
        }
        let file_name = file_name_of(path);
        let mime = mime.unwrap_or_else(|| {
            mime_guess::from_path(path)
                .first_or_octet_stream()
                .essence_str()
                .to_string()
        });
        let mut msg = self.new_message(to, None, None);
        let dest = self
            .store
            .lock()
            .unwrap()
            .attachment_path(&msg.id, &file_name);
        let sent = async {
            let (size, sha256) = copy_hashed(path, &dest).await?;
            msg.attachment = Some(Attachment {
                kind: AttachmentKind::from_mime(&mime),
                file_name,
                mime,
                size,
                sha256,
                duration_ms: None,
                path: Some(dest.clone()),
            });
            self.send_attachment(msg).await
        }
        .await;
        if sent.is_err() {
            // Nothing refers to the copy: do not leave it in the data folder.
            let _ = tokio::fs::remove_file(&dest).await;
        }
        sent
    }

    /// Send bytes produced in memory (a voice note, a pasted image). A copy is
    /// kept in the files directory so the sender can replay it later.
    pub async fn send_bytes(
        self: &Arc<Self>,
        to: Option<PeerId>,
        file_name: String,
        mime: String,
        bytes: Vec<u8>,
        duration_ms: Option<u32>,
    ) -> Result<Message, Error> {
        if bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
            return Err(Error::AttachmentTooLarge(bytes.len() as u64));
        }
        let mut attachment = Attachment {
            kind: AttachmentKind::from_mime(&mime),
            file_name,
            mime,
            size: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
            duration_ms,
            path: None,
        };
        let mut msg = self.new_message(to, None, None);
        let path =
            self.store
                .lock()
                .unwrap()
                .save_attachment(&msg.id, &attachment.file_name, &bytes)?;
        drop(bytes);
        attachment.path = Some(path);
        msg.attachment = Some(attachment);
        self.send_attachment(msg).await
    }

    /// Announce `msg` (which carries an attachment stored at
    /// `attachment.path`) to its targets and stream the bytes to each of
    /// them in the background.
    async fn send_attachment(self: &Arc<Self>, msg: Message) -> Result<Message, Error> {
        let att = msg.attachment.as_ref().expect("attachment");
        let path = att.path.clone().expect("attachment path");
        let total = att.size;
        let targets = self.route(&msg, None)?;
        // The header is already on its way: stream the bytes whatever happens
        // to the local history, or every peer would wait for chunks forever.
        for t in targets {
            let node = self.clone();
            let id = msg.id.clone();
            let path = path.clone();
            tokio::spawn(async move { node.stream_to(id, path, total, t).await });
        }
        self.store.lock().unwrap().append(msg.clone())?;
        Ok(msg)
    }

    /// Read the file in chunks and push them into one connection's data
    /// queue, waiting whenever the queue is full.
    async fn stream_to(&self, id: String, path: PathBuf, total: u64, target: Target) {
        let result: Result<(), Error> = async {
            let mut f = File::open(&path).await?;
            let mut buf = vec![0u8; CHUNK_BYTES];
            let mut sent = 0u64;
            let mut last_progress = 0u64;
            while sent < total {
                let want = (total - sent).min(CHUNK_BYTES as u64) as usize;
                let n = f.read(&mut buf[..want]).await?;
                if n == 0 {
                    return Err(Error::Other("file changed while sending".into()));
                }
                let pkt = Arc::new(Packet::chunk(&id, buf[..n].to_vec()));
                target
                    .data_tx
                    .send(pkt)
                    .await
                    .map_err(|_| Error::Unreachable(target.id.clone()))?;
                sent += n as u64;
                if sent - last_progress >= PROGRESS_STEP && sent < total {
                    last_progress = sent;
                    self.emit(Event::Transfer {
                        message_id: id.clone(),
                        peer: target.id.clone(),
                        direction: TransferDirection::Sending,
                        done: sent,
                        total,
                    })
                    .await;
                }
            }
            target
                .data_tx
                .send(Arc::new(Packet::frame(Frame::End { id: id.clone() })))
                .await
                .map_err(|_| Error::Unreachable(target.id.clone()))?;
            Ok(())
        }
        .await;
        match result {
            Ok(()) => {
                self.emit(Event::Transfer {
                    message_id: id,
                    peer: target.id,
                    direction: TransferDirection::Sending,
                    done: total,
                    total,
                })
                .await;
            }
            Err(e) => {
                tracing::warn!("sending attachment {id} to {} failed: {e}", target.id);
                // Wait for room: the queue is usually full right after a read
                // error, and a dropped Abort leaves the peer waiting forever.
                // Returns at once when the connection is gone.
                let _ = target
                    .data_tx
                    .send(Arc::new(Packet::frame(Frame::Abort { id: id.clone() })))
                    .await;
                self.emit(Event::TransferFailed {
                    message_id: id,
                    peer: target.id,
                    direction: TransferDirection::Sending,
                    reason: e.to_string(),
                })
                .await;
            }
        }
    }

    fn new_message(
        &self,
        to: Option<PeerId>,
        text: Option<String>,
        attachment: Option<Attachment>,
    ) -> Message {
        Message {
            id: uuid::Uuid::new_v4().to_string(),
            from: self.identity.id.clone(),
            from_name: self.identity.name.clone(),
            to,
            sent_at: now_ms(),
            text,
            attachment,
        }
    }

    /// Hand a message header to the connections that should carry it and
    /// return those that took it. `exclude` is the connection it arrived
    /// on, when forwarding.
    fn route(&self, message: &Message, exclude: Option<&PeerId>) -> Result<Vec<Target>, Error> {
        self.seen.lock().unwrap().insert(&message.id);
        let pkt = Arc::new(Packet::frame(Frame::Message {
            message: message.for_wire(),
        }));
        let candidates: Vec<Target> = {
            let inner = self.inner.lock().unwrap();
            let target_of = |c: &Conn| Target {
                id: c.info.id.clone(),
                tx: c.tx.clone(),
                data_tx: c.data_tx.clone(),
            };
            match &message.to {
                None => inner
                    .conns
                    .values()
                    .filter(|c| Some(&c.info.id) != exclude)
                    .map(target_of)
                    .collect(),
                Some(to) => {
                    if let Some(c) = inner.conns.get(to) {
                        vec![target_of(c)]
                    } else {
                        // Not directly connected: hand it to the relays.
                        inner
                            .conns
                            .values()
                            .filter(|c| c.info.relay && Some(&c.info.id) != exclude)
                            .map(target_of)
                            .collect()
                    }
                }
            }
        };
        if candidates.is_empty() {
            if let Some(to) = &message.to {
                if exclude.is_none() {
                    return Err(Error::Unreachable(to.clone()));
                }
            }
            return Ok(Vec::new());
        }
        let mut accepted = Vec::with_capacity(candidates.len());
        for t in candidates {
            // A slow peer must not stall the sender; drop if its outbox is full.
            if t.tx.try_send(pkt.clone()).is_err() {
                tracing::warn!("outbox full, dropping message for a slow peer");
            } else {
                accepted.push(t);
            }
        }
        Ok(accepted)
    }

    // ----- connecting ----------------------------------------------------

    /// Connect to a `host:port` (or `host`, using the default port). Returns
    /// once the handshake has succeeded; the connection then lives on its own.
    pub async fn connect(self: &Arc<Self>, target: &str) -> Result<(), Error> {
        let target = normalize_target(target);
        let addrs = resolve(&target).await?;
        let mut last = None;
        for addr in addrs {
            match self.connect_addr(addr).await {
                Ok(()) => return Ok(()),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| Error::Other(format!("cannot resolve {target}"))))
    }

    async fn connect_addr(self: &Arc<Self>, addr: SocketAddr) -> Result<(), Error> {
        let stream = timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
            .await
            .map_err(|_| Error::Other(format!("timed out connecting to {addr}")))??;
        tracing::debug!("outbound connection to {addr}");
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let node = self.clone();
        tokio::spawn(async move {
            if let Err(e) = node.run_connection(stream, true, Some(ready_tx)).await {
                tracing::debug!("connection to {addr} ended: {e}");
            }
        });
        match ready_rx.await {
            Ok(r) => r,
            Err(_) => Err(Error::Protocol("connection closed during handshake".into())),
        }
    }

    fn on_discovered(&self, d: Discovered) {
        let mut inner = self.inner.lock().unwrap();
        inner.known.insert(d.id.clone(), d);
    }

    /// The peer's mDNS announcement is gone: stop listing and dialing it. An
    /// existing connection is left alone; it ends on its own if the peer
    /// really left.
    fn on_lost(&self, id: &PeerId) {
        let mut inner = self.inner.lock().unwrap();
        inner.known.remove(id);
    }

    /// Peers that announced themselves with mDNS, connected or not.
    pub fn discovered(&self) -> Vec<FoundPeer> {
        let inner = self.inner.lock().unwrap();
        let mut v: Vec<FoundPeer> = inner
            .known
            .values()
            .filter_map(|d| {
                let addr = d.addrs.first()?;
                let mut p = FoundPeer {
                    id: d.id.clone(),
                    name: d.name.clone(),
                    addr: addr.to_string(),
                    relay: false,
                    via: FoundVia::Mdns,
                    connected: false,
                };
                overlay_conn(&inner, &mut p);
                Some(p)
            })
            .collect();
        v.sort_by_key(|p| p.name.to_lowercase());
        v
    }

    /// Actively look for ChatLocal on every `/24` this machine sits in, on
    /// our own port and the default one. Takes a few seconds. The result is
    /// merged with what mDNS already knows, one entry per peer.
    pub async fn scan_lan(&self) -> Vec<FoundPeer> {
        let subnets = crate::scan::subnets_of(&crate::local_ipv4());
        let ports = [self.port, DEFAULT_PORT];
        let scanned = crate::scan::scan(&subnets, &ports, &self.identity.id).await;

        let mut out = self.discovered();
        {
            let inner = self.inner.lock().unwrap();
            for mut p in scanned {
                if out.iter().any(|o| o.id == p.id) {
                    continue;
                }
                overlay_conn(&inner, &mut p);
                out.push(p);
            }
        }
        out.sort_by_key(|p| p.name.to_lowercase());
        out
    }

    /// Replace the addresses the reconnect loop dials. Existing connections
    /// are kept, including ones to addresses no longer in the list.
    pub fn set_manual_peers(&self, peers: Vec<String>) {
        *self.manual_peers.lock().unwrap() = peers;
    }

    /// Try every address we should be connected to but are not.
    async fn connect_pending(self: &Arc<Self>) {
        let mut targets: Vec<(Option<PeerId>, String)> = Vec::new();
        {
            let inner = self.inner.lock().unwrap();
            for (id, d) in &inner.known {
                if inner.conns.contains_key(id) {
                    continue;
                }
                // Tie-break: only the smaller id dials; the other side will
                // get connected to. Avoids both sides racing to connect.
                if self.identity.id > *id {
                    continue;
                }
                if let Some(a) = d.addrs.first() {
                    targets.push((Some(id.clone()), a.to_string()));
                }
            }
            for m in self.manual_peers.lock().unwrap().iter() {
                if !m.trim().is_empty() {
                    targets.push((None, normalize_target(m)));
                }
            }
        }
        for (peer_id, target) in targets {
            let addrs = match resolve(&target).await {
                Ok(a) => a,
                Err(e) => {
                    tracing::debug!("cannot resolve {target}: {e}");
                    continue;
                }
            };
            let Some(addr) = addrs.into_iter().next() else {
                continue;
            };
            {
                let mut inner = self.inner.lock().unwrap();
                if inner.connecting.contains(&addr) || inner.self_addrs.contains(&addr) {
                    continue;
                }
                if peer_id.is_none()
                    && inner
                        .conns
                        .values()
                        .any(|c| c.info.addr.as_deref() == Some(&addr.to_string()))
                {
                    continue;
                }
                inner.connecting.insert(addr);
            }
            let node = self.clone();
            tokio::spawn(async move {
                if let Err(e) = node.connect_addr(addr).await {
                    tracing::debug!("could not connect to {addr}: {e}");
                }
                node.inner.lock().unwrap().connecting.remove(&addr);
            });
        }
    }

    // ----- one connection ------------------------------------------------

    async fn run_connection(
        self: Arc<Self>,
        stream: TcpStream,
        initiated_by_me: bool,
        ready: Option<tokio::sync::oneshot::Sender<Result<(), Error>>>,
    ) -> Result<(), Error> {
        let mut ready = ready;
        let result = self
            .run_connection_inner(stream, initiated_by_me, &mut ready)
            .await;
        if let (Some(tx), Err(e)) = (ready.take(), &result) {
            let _ = tx.send(Err(Error::Protocol(e.to_string())));
        }
        result
    }

    async fn run_connection_inner(
        &self,
        stream: TcpStream,
        initiated_by_me: bool,
        ready: &mut Option<tokio::sync::oneshot::Sender<Result<(), Error>>>,
    ) -> Result<(), Error> {
        let remote = stream.peer_addr()?;
        let remote_target = remote;
        let _ = stream.set_nodelay(true);
        let (rd, wr) = stream.into_split();
        let mut rd = BufReader::with_capacity(64 * 1024, rd);
        let mut wr = BufWriter::with_capacity(64 * 1024, wr);

        write_packet(
            &mut wr,
            &Packet::frame(Frame::Hello {
                id: self.identity.id.clone(),
                name: self.identity.name.clone(),
                port: self.port,
                version: PROTOCOL_VERSION,
                relay: self.settings.relay,
            }),
        )
        .await?;

        let hello = timeout(HELLO_TIMEOUT, read_packet(&mut rd))
            .await
            .map_err(|_| Error::Protocol("handshake timed out".into()))??;
        let crate::wire::Hello {
            id,
            name,
            port,
            relay,
        } = hello.frame.into_hello()?;
        if id == self.identity.id {
            // Discovered our own announcement, or someone typed our own address.
            if initiated_by_me {
                self.inner.lock().unwrap().self_addrs.insert(remote_target);
            }
            if let Some(tx) = ready.take() {
                let _ = tx.send(Err(Error::Other("that address is this computer".into())));
            }
            return Ok(());
        }

        let serial = self.next_serial.fetch_add(1, Ordering::Relaxed);
        let (tx, mut rx) = mpsc::channel::<Arc<Packet>>(OUTBOX_CAPACITY);
        let (data_tx, mut data_rx) = mpsc::channel::<Arc<Packet>>(DATA_CAPACITY);
        let cancel = CancellationToken::new();
        let info = PeerInfo {
            id: id.clone(),
            name: name.clone(),
            addr: Some(SocketAddr::new(remote.ip(), port).to_string()),
            relay,
            direct: true,
            via: None,
        };

        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(existing) = inner.conns.get(&id) {
                // Both sides may dial at once. Keep the connection dialed by
                // the smaller id; a repeat in the same direction replaces.
                let i_am_smaller = self.identity.id < id;
                let keep_new =
                    existing.initiated_by_me == initiated_by_me || initiated_by_me == i_am_smaller;
                if !keep_new {
                    tracing::debug!("dropping duplicate connection to {name}");
                    if let Some(tx) = ready.take() {
                        let _ = tx.send(Ok(()));
                    }
                    return Ok(());
                }
                if let Some(old) = inner.conns.remove(&id) {
                    old.cancel.cancel();
                }
            }
            inner.conns.insert(
                id.clone(),
                Conn {
                    serial,
                    info: info.clone(),
                    initiated_by_me,
                    tx: tx.clone(),
                    data_tx,
                    cancel: cancel.clone(),
                },
            );
        }
        tracing::info!("connected to {name} ({remote})");
        if let Some(tx) = ready.take() {
            let _ = tx.send(Ok(()));
        }
        self.recompute_via_relay().await;
        self.emit(Event::PeerOnline { peer: info.clone() }).await;
        if self.settings.relay {
            self.broadcast_roster();
        }

        // Writer.
        let writer_cancel = cancel.clone();
        // Control frames win whenever both queues have something, so a text
        // message never waits behind more than the chunk being written.
        let writer = tokio::spawn(async move {
            loop {
                let pkt = tokio::select! {
                    biased;
                    _ = writer_cancel.cancelled() => break,
                    next = rx.recv() => match next {
                        Some(pkt) => pkt,
                        None => break,
                    },
                    next = data_rx.recv() => match next {
                        Some(pkt) => pkt,
                        None => break,
                    },
                };
                match write_packet(&mut wr, &pkt).await {
                    Ok(()) => {}
                    // Refused before any byte went out: skip it, keep the
                    // connection.
                    Err(Error::FrameTooLarge(n)) => {
                        tracing::warn!("dropping a frame of {n} bytes that exceeds the limit");
                    }
                    Err(e) => {
                        tracing::debug!("write failed: {e}");
                        break;
                    }
                }
            }
            let _ = wr.shutdown().await;
        });

        // Reader.
        let mut incoming: HashMap<String, Incoming> = HashMap::new();
        let result: Result<(), Error> = async {
            let mut ping = tokio::time::interval(PING_INTERVAL);
            ping.tick().await;
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return Ok(()),
                    _ = self.shutdown.cancelled() => return Ok(()),
                    _ = ping.tick() => {
                        let _ = tx.try_send(Arc::new(Packet::frame(Frame::Ping)));
                        self.expire_incoming(&id, &mut incoming).await;
                    }
                    pkt = timeout(READ_TIMEOUT, read_packet(&mut rd)) => {
                        let pkt = pkt.map_err(|_| Error::Protocol("peer went silent".into()))??;
                        self.handle_packet(&id, &tx, pkt, &mut incoming).await;
                    }
                }
            }
        }
        .await;

        // Whatever was still streaming in is lost; tell everyone downstream.
        for (mid, inc) in incoming.drain() {
            self.fail_incoming(&id, mid, inc, "connection closed").await;
        }

        cancel.cancel();
        let _ = writer.await;

        let removed = {
            let mut inner = self.inner.lock().unwrap();
            let is_current = inner.conns.get(&id).map(|c| c.serial) == Some(serial);
            if is_current {
                inner.conns.remove(&id);
                inner.rosters.remove(&id);
            }
            is_current
        };
        if removed {
            tracing::info!("disconnected from {name}");
            self.emit(Event::PeerOffline { id: id.clone() }).await;
            self.recompute_via_relay().await;
            if self.settings.relay {
                self.broadcast_roster();
            }
        }
        result
    }

    async fn handle_packet(
        &self,
        from: &PeerId,
        tx: &mpsc::Sender<Arc<Packet>>,
        pkt: Packet,
        incoming: &mut HashMap<String, Incoming>,
    ) {
        match pkt.frame {
            Frame::Ping => {
                let _ = tx.try_send(Arc::new(Packet::frame(Frame::Pong)));
            }
            Frame::Pong | Frame::Hello { .. } => {}
            Frame::Roster { peers } => {
                self.inner
                    .lock()
                    .unwrap()
                    .rosters
                    .insert(from.clone(), peers);
                self.recompute_via_relay().await;
            }
            Frame::Message { message } => {
                self.handle_message(from, message, incoming).await;
            }
            Frame::Chunk { id } => {
                let pkt = Arc::new(Packet {
                    frame: Frame::Chunk { id: id.clone() },
                    payload: pkt.payload,
                });
                self.handle_chunk(from, id, pkt, incoming).await;
            }
            Frame::End { id } => {
                if let Some(inc) = incoming.remove(&id) {
                    self.finish_incoming(from, id, inc).await;
                }
            }
            Frame::Abort { id } => {
                if let Some(inc) = incoming.remove(&id) {
                    self.fail_incoming(from, id, inc, "the sender gave up")
                        .await;
                }
            }
        }
    }

    async fn handle_message(
        &self,
        from: &PeerId,
        mut message: Message,
        incoming: &mut HashMap<String, Incoming>,
    ) {
        if message.from == self.identity.id {
            return;
        }
        if !self.seen.lock().unwrap().insert(&message.id) {
            return;
        }
        let for_me = match &message.to {
            None => true,
            Some(to) => *to == self.identity.id,
        };
        let forward_it = self.settings.relay && (message.to.is_none() || !for_me);

        let Some(att) = message.attachment.as_mut() else {
            // Plain text: deliver and/or forward right away.
            if for_me {
                if let Err(e) = self.store.lock().unwrap().append(message.clone()) {
                    tracing::warn!("could not persist message: {e}");
                }
                self.emit(Event::Message {
                    message: message.clone(),
                })
                .await;
            }
            if forward_it {
                if let Err(e) = self.route(&message, Some(from)) {
                    tracing::debug!("relay could not forward: {e}");
                }
            }
            return;
        };

        // An attachment: the bytes follow as chunks. Set up where they go.
        att.path = None;
        let total = att.size;
        let file_name = att.file_name.clone();
        if total > MAX_ATTACHMENT_BYTES {
            tracing::warn!(
                "ignoring attachment from {} larger than the limit",
                message.from_name
            );
            return;
        }
        if incoming.len() >= MAX_INCOMING_PER_CONN {
            tracing::warn!("too many transfers in flight from {}", message.from_name);
            return;
        }
        let file = if for_me {
            let partial = self
                .store
                .lock()
                .unwrap()
                .partial_path(&message.id, &file_name);
            match File::create(&partial).await {
                Ok(f) => Some((f, partial)),
                Err(e) => {
                    tracing::warn!("could not create attachment file: {e}");
                    None
                }
            }
        } else {
            None
        };
        let forward = if forward_it {
            self.route(&message, Some(from)).unwrap_or_default()
        } else {
            Vec::new()
        };
        if file.is_none() && forward.is_empty() {
            return;
        }
        incoming.insert(
            message.id.clone(),
            Incoming {
                message,
                total,
                received: 0,
                hasher: Sha256::new(),
                file,
                forward,
                last_progress: 0,
                last_activity: Instant::now(),
            },
        );
    }

    /// Drop transfers whose sender stopped sending without an `End` or
    /// `Abort` (lost frame, stalled peer), freeing their slot and file.
    async fn expire_incoming(&self, from: &PeerId, incoming: &mut HashMap<String, Incoming>) {
        let stale: Vec<String> = incoming
            .iter()
            .filter(|(_, inc)| inc.last_activity.elapsed() > INCOMING_IDLE_TIMEOUT)
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            if let Some(inc) = incoming.remove(&id) {
                self.fail_incoming(from, id, inc, "transfer stalled").await;
            }
        }
    }

    async fn handle_chunk(
        &self,
        from: &PeerId,
        id: String,
        pkt: Arc<Packet>,
        incoming: &mut HashMap<String, Incoming>,
    ) {
        let Some(inc) = incoming.get_mut(&id) else {
            return; // Unknown or already failed: skip the bytes.
        };
        let n = pkt.payload.len() as u64;
        if inc.received + n > inc.total {
            let inc = incoming.remove(&id).unwrap();
            self.fail_incoming(from, id, inc, "more bytes than announced")
                .await;
            return;
        }
        if let Some((f, _)) = inc.file.as_mut() {
            if let Err(e) = f.write_all(&pkt.payload).await {
                let inc = incoming.remove(&id).unwrap();
                self.fail_incoming(from, id, inc, &format!("could not write file: {e}"))
                    .await;
                return;
            }
        }
        inc.hasher.update(&pkt.payload);
        inc.received += n;
        inc.last_activity = Instant::now();
        forward_packet(&mut inc.forward, &pkt).await;
        if inc.file.is_some() && inc.received - inc.last_progress >= PROGRESS_STEP {
            inc.last_progress = inc.received;
            self.emit(Event::Transfer {
                message_id: id,
                peer: from.clone(),
                direction: TransferDirection::Receiving,
                done: inc.received,
                total: inc.total,
            })
            .await;
        }
    }

    /// Every chunk arrived: verify, move the file into place, deliver.
    async fn finish_incoming(&self, from: &PeerId, id: String, mut inc: Incoming) {
        forward_packet(
            &mut inc.forward,
            &Arc::new(Packet::frame(Frame::End { id: id.clone() })),
        )
        .await;
        let Some((mut f, partial)) = inc.file.take() else {
            return; // Relay only: nothing to keep.
        };
        let att = inc.message.attachment.as_mut().expect("attachment");
        let digest = hex::encode(std::mem::take(&mut inc.hasher).finalize());
        let verified: Result<(), String> = async {
            if inc.received != inc.total {
                return Err(format!("received {} of {} bytes", inc.received, inc.total));
            }
            if digest != att.sha256 {
                return Err("checksum mismatch".into());
            }
            f.flush().await.map_err(|e| e.to_string())?;
            drop(f);
            let final_path = self
                .store
                .lock()
                .unwrap()
                .attachment_path(&inc.message.id, &att.file_name);
            tokio::fs::rename(&partial, &final_path)
                .await
                .map_err(|e| e.to_string())?;
            att.path = Some(final_path);
            Ok(())
        }
        .await;
        if let Err(reason) = verified {
            tracing::warn!(
                "attachment from {} failed verification: {reason}",
                inc.message.from_name
            );
            let _ = tokio::fs::remove_file(&partial).await;
            self.emit(Event::TransferFailed {
                message_id: id,
                peer: from.clone(),
                direction: TransferDirection::Receiving,
                reason,
            })
            .await;
            return;
        }
        if let Err(e) = self.store.lock().unwrap().append(inc.message.clone()) {
            tracing::warn!("could not persist message: {e}");
        }
        self.emit(Event::Transfer {
            message_id: id,
            peer: from.clone(),
            direction: TransferDirection::Receiving,
            done: inc.total,
            total: inc.total,
        })
        .await;
        self.emit(Event::Message {
            message: inc.message,
        })
        .await;
    }

    /// Drop a half-received attachment: delete the partial file and tell
    /// downstream peers to do the same.
    async fn fail_incoming(&self, from: &PeerId, id: String, mut inc: Incoming, reason: &str) {
        // Abort goes on the control queue: it overtakes chunks still queued
        // for the target (which then ignores them) and never waits on a
        // full data queue, so a dead upstream is torn down without delay.
        let abort = Arc::new(Packet::frame(Frame::Abort { id: id.clone() }));
        for t in inc.forward.drain(..) {
            let _ = t.tx.try_send(abort.clone());
        }
        if let Some((f, partial)) = inc.file.take() {
            drop(f);
            let _ = tokio::fs::remove_file(&partial).await;
            tracing::warn!(
                "attachment from {} not received: {reason}",
                inc.message.from_name
            );
            self.emit(Event::TransferFailed {
                message_id: id,
                peer: from.clone(),
                direction: TransferDirection::Receiving,
                reason: reason.to_string(),
            })
            .await;
        }
    }

    /// Push our direct peer list to everyone (relay mode only).
    fn broadcast_roster(&self) {
        let inner = self.inner.lock().unwrap();
        let peers: Vec<RosterEntry> = inner
            .conns
            .values()
            .map(|c| RosterEntry {
                id: c.info.id.clone(),
                name: c.info.name.clone(),
            })
            .collect();
        let pkt = Arc::new(Packet::frame(Frame::Roster { peers }));
        for c in inner.conns.values() {
            let _ = c.tx.try_send(pkt.clone());
        }
    }

    /// Rebuild the set of peers reachable only through a relay and emit the
    /// difference as online/offline events.
    async fn recompute_via_relay(&self) {
        let (gone, appeared) = {
            let mut inner = self.inner.lock().unwrap();
            let mut fresh: HashMap<PeerId, PeerInfo> = HashMap::new();
            for (relay_id, roster) in &inner.rosters {
                for e in roster {
                    if e.id == self.identity.id || inner.conns.contains_key(&e.id) {
                        continue;
                    }
                    fresh.entry(e.id.clone()).or_insert(PeerInfo {
                        id: e.id.clone(),
                        name: e.name.clone(),
                        addr: None,
                        relay: false,
                        direct: false,
                        via: Some(relay_id.clone()),
                    });
                }
            }
            let gone: Vec<PeerId> = inner
                .via_relay
                .keys()
                .filter(|k| !fresh.contains_key(*k))
                .cloned()
                .collect();
            let appeared: Vec<PeerInfo> = fresh
                .values()
                .filter(|p| inner.via_relay.get(&p.id) != Some(*p))
                .cloned()
                .collect();
            inner.via_relay = fresh;
            (gone, appeared)
        };
        for id in gone {
            self.emit(Event::PeerOffline { id }).await;
        }
        for peer in appeared {
            self.emit(Event::PeerOnline { peer }).await;
        }
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

/// Push one packet into every target's data queue, waiting up to
/// [`FORWARD_TIMEOUT`] for each. A target that is gone or too slow is
/// dropped from the list and told to discard the transfer.
async fn forward_packet(targets: &mut Vec<Target>, pkt: &Arc<Packet>) {
    let mut i = 0;
    while i < targets.len() {
        match timeout(FORWARD_TIMEOUT, targets[i].data_tx.send(pkt.clone())).await {
            Ok(Ok(())) => i += 1,
            _ => {
                let t = targets.swap_remove(i);
                tracing::debug!("stopped forwarding a transfer to {}", t.id);
                // Whatever frame was lost (a chunk, the End, or an Abort),
                // the target must discard the transfer; a repeat Abort is
                // harmless.
                if let Frame::Chunk { id } | Frame::End { id } | Frame::Abort { id } = &pkt.frame {
                    let _ =
                        t.tx.try_send(Arc::new(Packet::frame(Frame::Abort { id: id.clone() })));
                }
            }
        }
    }
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into())
}

/// `host` -> `host:DEFAULT_PORT`; `host:port` unchanged; IPv6 needs brackets.
/// What a live connection knows about a found peer beats what discovery or
/// a probe said: its current name, the address we actually talk to, and
/// whether it is a relay.
fn overlay_conn(inner: &Inner, p: &mut FoundPeer) {
    if let Some(c) = inner.conns.get(&p.id) {
        p.connected = true;
        p.name = c.info.name.clone();
        if let Some(a) = &c.info.addr {
            p.addr = a.clone();
        }
        p.relay = c.info.relay;
    }
}

pub fn normalize_target(s: &str) -> String {
    let s = s.trim();
    if s.starts_with('[') {
        if s.contains("]:") {
            return s.to_string();
        }
        return format!("{s}:{DEFAULT_PORT}");
    }
    match s.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') && port.parse::<u16>().is_ok() => s.to_string(),
        _ => format!("{s}:{DEFAULT_PORT}"),
    }
}

async fn resolve(target: &str) -> Result<Vec<SocketAddr>, Error> {
    let t = target.to_string();
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host(&t).await?.collect();
    if addrs.is_empty() {
        return Err(Error::Other(format!("cannot resolve {target}")));
    }
    Ok(addrs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_targets() {
        assert_eq!(
            normalize_target("10.0.0.5"),
            format!("10.0.0.5:{DEFAULT_PORT}")
        );
        assert_eq!(normalize_target("10.0.0.5:5000"), "10.0.0.5:5000");
        assert_eq!(
            normalize_target(" pc-dapur.local "),
            format!("pc-dapur.local:{DEFAULT_PORT}")
        );
        assert_eq!(
            normalize_target("[fe80::1]"),
            format!("[fe80::1]:{DEFAULT_PORT}")
        );
        assert_eq!(normalize_target("[fe80::1]:9"), "[fe80::1]:9");
    }
}
