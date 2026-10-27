//! The node: listener, outgoing connections, discovery glue, relaying and the
//! public send API.

use crate::discovery::{Discovered, Discovery};
use crate::store::Store;
use crate::types::*;
use crate::wire::{read_packet, write_packet, Frame, Packet, RosterEntry};
use crate::Error;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncWriteExt, BufReader, BufWriter};
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
const OUTBOX_CAPACITY: usize = 256;

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
    tx: mpsc::Sender<Arc<Packet>>,
    cancel: CancellationToken,
}

#[derive(Default)]
struct Inner {
    conns: HashMap<PeerId, Conn>,
    /// Last roster received from each relay we are connected to.
    rosters: HashMap<PeerId, Vec<RosterEntry>>,
    /// Peers reachable only through a relay, derived from `rosters`.
    via_relay: HashMap<PeerId, PeerInfo>,
    /// Addresses learned from discovery, by peer id.
    known_addrs: HashMap<PeerId, Vec<SocketAddr>>,
    /// Addresses with a connection attempt in flight.
    connecting: HashSet<SocketAddr>,
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
    pub settings: Settings,
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
                                    Some(d) => n.on_discovered(d).await,
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
        let msg = self.new_message(to, Some(text), None);
        self.route(msg.clone(), Vec::new(), None).await?;
        self.store.lock().unwrap().append(msg.clone())?;
        Ok(msg)
    }

    /// Send a file from disk. The original file stays where it is.
    pub async fn send_file(&self, to: Option<PeerId>, path: &Path) -> Result<Message, Error> {
        let meta = tokio::fs::metadata(path).await?;
        if !meta.is_file() {
            return Err(Error::Other("not a file".into()));
        }
        if meta.len() > MAX_ATTACHMENT_BYTES {
            return Err(Error::AttachmentTooLarge(meta.len()));
        }
        let bytes = tokio::fs::read(path).await?;
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".into());
        let mime = mime_guess::from_path(path)
            .first_or_octet_stream()
            .essence_str()
            .to_string();
        let attachment = Attachment {
            kind: AttachmentKind::from_mime(&mime),
            file_name,
            mime,
            size: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
            duration_ms: None,
            path: Some(path.to_path_buf()),
        };
        let msg = self.new_message(to, None, Some(attachment));
        self.route(msg.clone(), bytes, None).await?;
        self.store.lock().unwrap().append(msg.clone())?;
        Ok(msg)
    }

    /// Send bytes produced in memory (a voice note, a pasted image). A copy is
    /// kept in the files directory so the sender can replay it later.
    pub async fn send_bytes(
        &self,
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
        attachment.path = Some(path);
        msg.attachment = Some(attachment);
        self.route(msg.clone(), bytes, None).await?;
        self.store.lock().unwrap().append(msg.clone())?;
        Ok(msg)
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

    /// Hand a message to the connections that should carry it. `exclude` is
    /// the connection it arrived on, when forwarding.
    async fn route(
        &self,
        message: Message,
        payload: Vec<u8>,
        exclude: Option<&PeerId>,
    ) -> Result<(), Error> {
        self.seen.lock().unwrap().insert(&message.id);
        let pkt = Arc::new(Packet {
            frame: Frame::Message {
                message: message.for_wire(),
            },
            payload,
        });
        let targets: Vec<mpsc::Sender<Arc<Packet>>> = {
            let inner = self.inner.lock().unwrap();
            match &message.to {
                None => inner
                    .conns
                    .values()
                    .filter(|c| Some(&c.info.id) != exclude)
                    .map(|c| c.tx.clone())
                    .collect(),
                Some(to) => {
                    if let Some(c) = inner.conns.get(to) {
                        vec![c.tx.clone()]
                    } else {
                        // Not directly connected: hand it to the relays.
                        inner
                            .conns
                            .values()
                            .filter(|c| c.info.relay && Some(&c.info.id) != exclude)
                            .map(|c| c.tx.clone())
                            .collect()
                    }
                }
            }
        };
        if targets.is_empty() {
            if let Some(to) = &message.to {
                if exclude.is_none() {
                    return Err(Error::Unreachable(to.clone()));
                }
            }
            return Ok(());
        }
        for tx in targets {
            // A slow peer must not stall the sender; drop if its outbox is full.
            if tx.try_send(pkt.clone()).is_err() {
                tracing::warn!("outbox full, dropping message for a slow peer");
            }
        }
        Ok(())
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

    async fn on_discovered(&self, d: Discovered) {
        let mut inner = self.inner.lock().unwrap();
        inner.known_addrs.insert(d.id.clone(), d.addrs);
    }

    /// Try every address we should be connected to but are not.
    async fn connect_pending(self: &Arc<Self>) {
        let mut targets: Vec<(Option<PeerId>, String)> = Vec::new();
        {
            let inner = self.inner.lock().unwrap();
            for (id, addrs) in &inner.known_addrs {
                if inner.conns.contains_key(id) {
                    continue;
                }
                // Tie-break: only the smaller id dials; the other side will
                // get connected to. Avoids both sides racing to connect.
                if self.identity.id > *id {
                    continue;
                }
                if let Some(a) = addrs.first() {
                    targets.push((Some(id.clone()), a.to_string()));
                }
            }
            for m in &self.settings.manual_peers {
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
                if inner.connecting.contains(&addr) {
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
        let Frame::Hello {
            id,
            name,
            port,
            version,
            relay,
        } = hello.frame
        else {
            return Err(Error::Protocol("expected hello".into()));
        };
        if version != PROTOCOL_VERSION {
            return Err(Error::Protocol(format!(
                "peer speaks protocol v{version}, we speak v{PROTOCOL_VERSION}"
            )));
        }
        if id == self.identity.id {
            // Discovered our own announcement, or someone typed our own address.
            if let Some(tx) = ready.take() {
                let _ = tx.send(Err(Error::Other("that address is this computer".into())));
            }
            return Ok(());
        }

        let serial = self.next_serial.fetch_add(1, Ordering::Relaxed);
        let (tx, mut rx) = mpsc::channel::<Arc<Packet>>(OUTBOX_CAPACITY);
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
        let writer = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = writer_cancel.cancelled() => break,
                    next = rx.recv() => match next {
                        Some(pkt) => {
                            if let Err(e) = write_packet(&mut wr, &pkt).await {
                                tracing::debug!("write failed: {e}");
                                break;
                            }
                        }
                        None => break,
                    }
                }
            }
            let _ = wr.shutdown().await;
        });

        // Reader.
        let result: Result<(), Error> = async {
            let mut ping = tokio::time::interval(PING_INTERVAL);
            ping.tick().await;
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => return Ok(()),
                    _ = self.shutdown.cancelled() => return Ok(()),
                    _ = ping.tick() => {
                        let _ = tx.try_send(Arc::new(Packet::frame(Frame::Ping)));
                    }
                    pkt = timeout(READ_TIMEOUT, read_packet(&mut rd)) => {
                        let pkt = pkt.map_err(|_| Error::Protocol("peer went silent".into()))??;
                        self.handle_packet(&id, &tx, pkt).await;
                    }
                }
            }
        }
        .await;

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

    async fn handle_packet(&self, from: &PeerId, tx: &mpsc::Sender<Arc<Packet>>, pkt: Packet) {
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
                self.handle_message(from, message, pkt.payload).await;
            }
        }
    }

    async fn handle_message(&self, from: &PeerId, mut message: Message, payload: Vec<u8>) {
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

        if for_me {
            let mut deliver = true;
            if let Some(att) = message.attachment.as_mut() {
                if payload.len() as u64 != att.size || sha256_hex(&payload) != att.sha256 {
                    tracing::warn!("attachment from {} failed verification", message.from_name);
                    deliver = false;
                } else {
                    let saved = self.store.lock().unwrap().save_attachment(
                        &message.id,
                        &att.file_name,
                        &payload,
                    );
                    match saved {
                        Ok(p) => att.path = Some(p),
                        Err(e) => {
                            tracing::warn!("could not save attachment: {e}");
                            deliver = false;
                        }
                    }
                }
            }
            if deliver {
                if let Err(e) = self.store.lock().unwrap().append(message.clone()) {
                    tracing::warn!("could not persist message: {e}");
                }
                self.emit(Event::Message {
                    message: message.clone(),
                })
                .await;
            }
        }

        if self.settings.relay && (message.to.is_none() || !for_me) {
            if let Err(e) = self.route(message, payload, Some(from)).await {
                tracing::debug!("relay could not forward: {e}");
            }
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

/// `host` -> `host:DEFAULT_PORT`; `host:port` unchanged; IPv6 needs brackets.
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
