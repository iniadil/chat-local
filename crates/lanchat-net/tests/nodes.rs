//! End-to-end tests over loopback: no mDNS, nodes are wired by address.

use lanchat_net::*;
use std::time::Duration;
use tokio::sync::mpsc::Receiver;
use tokio::time::timeout;

async fn spawn(
    name: &str,
    relay: bool,
    manual: Vec<String>,
) -> (std::sync::Arc<Node>, Receiver<Event>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let (node, mut rx) = Node::start(NodeConfig {
        identity: Identity {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
        },
        settings: Settings {
            display_name: name.into(),
            port: 0,
            discovery: false,
            relay,
            manual_peers: manual,
        },
        data_dir: dir.path().to_path_buf(),
    })
    .await
    .unwrap();
    // Consume the Ready event.
    match rx.recv().await {
        Some(Event::Ready { .. }) => {}
        other => panic!("expected Ready, got {other:?}"),
    }
    (node, rx, dir)
}

async fn wait_for<F: Fn(&Event) -> bool>(rx: &mut Receiver<Event>, what: &str, pred: F) -> Event {
    loop {
        let ev = timeout(Duration::from_secs(10), rx.recv())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
            .expect("event channel closed");
        if pred(&ev) {
            return ev;
        }
    }
}

#[tokio::test]
async fn two_nodes_exchange_text_and_files() {
    let (a, mut a_rx, _da) = spawn("Alice", false, vec![]).await;
    let (b, mut b_rx, _db) = spawn("Bob", false, vec![format!("127.0.0.1:{}", a.port())]).await;

    wait_for(
        &mut a_rx,
        "Bob online at Alice",
        |e| matches!(e, Event::PeerOnline { peer } if peer.name == "Bob"),
    )
    .await;
    wait_for(
        &mut b_rx,
        "Alice online at Bob",
        |e| matches!(e, Event::PeerOnline { peer } if peer.name == "Alice"),
    )
    .await;
    assert_eq!(a.peers().len(), 1);
    assert_eq!(b.peers().len(), 1);

    // Room message.
    let sent = a.send_text(None, "halo semua".into()).await.unwrap();
    let got = wait_for(&mut b_rx, "room text at Bob", |e| {
        matches!(e, Event::Message { .. })
    })
    .await;
    let Event::Message { message } = got else {
        unreachable!()
    };
    assert_eq!(message.id, sent.id);
    assert_eq!(message.text.as_deref(), Some("halo semua"));
    assert_eq!(message.to, None);

    // Direct message with an attachment.
    let payload: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let sent = b
        .send_bytes(
            Some(a.identity.id.clone()),
            "scan.png".into(),
            "image/png".into(),
            payload.clone(),
            None,
        )
        .await
        .unwrap();
    assert!(sent
        .attachment
        .as_ref()
        .unwrap()
        .path
        .as_ref()
        .unwrap()
        .exists());
    let got = wait_for(&mut a_rx, "attachment at Alice", |e| {
        matches!(e, Event::Message { .. })
    })
    .await;
    let Event::Message { message } = got else {
        unreachable!()
    };
    assert_eq!(message.to.as_deref(), Some(a.identity.id.as_str()));
    let att = message.attachment.unwrap();
    assert_eq!(att.kind, AttachmentKind::Image);
    assert_eq!(att.size, payload.len() as u64);
    let saved = std::fs::read(att.path.unwrap()).unwrap();
    assert_eq!(saved, payload);

    // History survived on both sides.
    assert_eq!(a.messages().len(), 2);
    assert_eq!(b.messages().len(), 2);

    // Disconnect is noticed.
    b.shutdown();
    wait_for(&mut a_rx, "Bob offline at Alice", |e| {
        matches!(e, Event::PeerOffline { .. })
    })
    .await;
    assert!(a.peers().is_empty());
}

#[tokio::test]
async fn relay_connects_clients_that_cannot_see_each_other() {
    let (hub, mut hub_rx, _dh) = spawn("Hub", true, vec![]).await;
    let addr = format!("127.0.0.1:{}", hub.port());
    let (a, mut a_rx, _da) = spawn("Alice", false, vec![addr.clone()]).await;
    let (c, mut c_rx, _dc) = spawn("Cici", false, vec![addr]).await;

    wait_for(
        &mut hub_rx,
        "Alice at hub",
        |e| matches!(e, Event::PeerOnline { peer } if peer.name == "Alice"),
    )
    .await;
    wait_for(
        &mut hub_rx,
        "Cici at hub",
        |e| matches!(e, Event::PeerOnline { peer } if peer.name == "Cici"),
    )
    .await;
    // Alice learns about Cici through the hub's roster, not a direct link.
    let ev = wait_for(
        &mut a_rx,
        "Cici via hub at Alice",
        |e| matches!(e, Event::PeerOnline { peer } if peer.name == "Cici"),
    )
    .await;
    let Event::PeerOnline { peer } = ev else {
        unreachable!()
    };
    assert!(!peer.direct);
    assert_eq!(peer.via.as_deref(), Some(hub.identity.id.as_str()));
    wait_for(
        &mut c_rx,
        "Alice via hub at Cici",
        |e| matches!(e, Event::PeerOnline { peer } if peer.name == "Alice"),
    )
    .await;

    // Room message reaches both the hub and the other client.
    a.send_text(None, "ping".into()).await.unwrap();
    wait_for(
        &mut hub_rx,
        "room at hub",
        |e| matches!(e, Event::Message { message } if message.text.as_deref() == Some("ping")),
    )
    .await;
    wait_for(
        &mut c_rx,
        "room at Cici",
        |e| matches!(e, Event::Message { message } if message.text.as_deref() == Some("ping")),
    )
    .await;

    // Direct message is forwarded by the hub and not delivered to the hub.
    c.send_text(Some(a.identity.id.clone()), "rahasia".into())
        .await
        .unwrap();
    wait_for(
        &mut a_rx,
        "dm at Alice",
        |e| matches!(e, Event::Message { message } if message.text.as_deref() == Some("rahasia")),
    )
    .await;
    assert!(!hub
        .messages()
        .iter()
        .any(|m| m.text.as_deref() == Some("rahasia")));

    // When Cici leaves, Alice sees her go.
    c.shutdown();
    wait_for(
        &mut a_rx,
        "Cici offline at Alice",
        |e| matches!(e, Event::PeerOffline { id } if *id == c.identity.id),
    )
    .await;

    // A DM to someone unreachable is an error, not a silent drop.
    let (lonely, _rx, _d) = spawn("Lonely", false, vec![]).await;
    assert!(matches!(
        lonely.send_text(Some("nobody".into()), "hi".into()).await,
        Err(Error::Unreachable(_))
    ));
}

#[tokio::test]
async fn simultaneous_dial_keeps_one_connection() {
    let (a, mut a_rx, _da) = spawn("A", false, vec![]).await;
    let (b, mut b_rx, _db) = spawn("B", false, vec![]).await;
    let to_b = format!("127.0.0.1:{}", b.port());
    let to_a = format!("127.0.0.1:{}", a.port());
    let (ra, rb) = tokio::join!(a.connect(&to_b), b.connect(&to_a));
    // Both calls return once their connection ends; the loser ends immediately.
    let _ = (ra, rb);
    wait_for(&mut a_rx, "B online", |e| {
        matches!(e, Event::PeerOnline { .. })
    })
    .await;
    wait_for(&mut b_rx, "A online", |e| {
        matches!(e, Event::PeerOnline { .. })
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(a.peers().len(), 1);
    assert_eq!(b.peers().len(), 1);
}

/// A file far larger than one chunk crosses a relay intact, the relay keeps
/// nothing on disk for a direct message, and text sent during the transfer
/// is not held behind it.
#[tokio::test]
async fn large_file_streams_through_relay_without_blocking_text() {
    let (hub, mut hub_rx, dh) = spawn("Hub", true, vec![]).await;
    let addr = format!("127.0.0.1:{}", hub.port());
    let (a, mut a_rx, _da) = spawn("Alice", false, vec![addr.clone()]).await;
    let (c, mut c_rx, _dc) = spawn("Cici", false, vec![addr]).await;
    wait_for(
        &mut hub_rx,
        "Cici at hub",
        |e| matches!(e, Event::PeerOnline { peer } if peer.name == "Cici"),
    )
    .await;
    wait_for(
        &mut a_rx,
        "Cici via hub at Alice",
        |e| matches!(e, Event::PeerOnline { peer } if peer.name == "Cici"),
    )
    .await;
    wait_for(
        &mut c_rx,
        "Alice via hub at Cici",
        |e| matches!(e, Event::PeerOnline { peer } if peer.name == "Alice"),
    )
    .await;

    // 6 MB: many chunks, several progress events.
    let src = _da.path().join("big.bin");
    let payload: Vec<u8> = (0..6 * 1024 * 1024u32).map(|i| (i % 253) as u8).collect();
    std::fs::write(&src, &payload).unwrap();
    let sent = a
        .send_file(Some(c.identity.id.clone()), &src)
        .await
        .unwrap();
    // Text right behind the file must arrive before the file is complete.
    a.send_text(Some(c.identity.id.clone()), "masih hidup?".into())
        .await
        .unwrap();

    let mut text_seen = false;
    let mut progress_seen = false;
    let file_msg = loop {
        let ev = timeout(Duration::from_secs(20), c_rx.recv())
            .await
            .expect("timed out at Cici")
            .expect("closed");
        match ev {
            Event::Message { message } if message.text.is_some() => {
                text_seen = true;
            }
            Event::Transfer { done, total, .. } => {
                assert!(done <= total);
                if done < total {
                    progress_seen = true;
                }
            }
            Event::Message { message } => {
                assert!(text_seen, "text was stuck behind the file transfer");
                break message;
            }
            Event::TransferFailed { reason, .. } => panic!("transfer failed: {reason}"),
            _ => {}
        }
    };
    assert!(progress_seen);
    assert_eq!(file_msg.id, sent.id);
    let att = file_msg.attachment.unwrap();
    assert_eq!(att.size, payload.len() as u64);
    assert_eq!(std::fs::read(att.path.unwrap()).unwrap(), payload);

    // Sender saw completion too.
    wait_for(&mut a_rx, "send complete at Alice", |e| {
        matches!(e, Event::Transfer { done, total, direction: TransferDirection::Sending, .. } if done == total)
    })
    .await;

    // The hub forwarded without keeping anything.
    let hub_files: Vec<_> = std::fs::read_dir(dh.path().join("files"))
        .unwrap()
        .flatten()
        .collect();
    assert!(hub_files.is_empty(), "hub kept {:?}", hub_files);
    assert!(hub.messages().is_empty());
}

/// If the sender goes away mid-transfer, the receiver drops the partial file
/// and reports the failure instead of delivering a broken message.
#[tokio::test]
async fn interrupted_transfer_leaves_no_partial_file() {
    let (a, mut a_rx, da) = spawn("Alice", false, vec![]).await;
    let (b, mut b_rx, _db) = spawn("Bob", false, vec![format!("127.0.0.1:{}", a.port())]).await;
    wait_for(&mut a_rx, "Bob at Alice", |e| {
        matches!(e, Event::PeerOnline { .. })
    })
    .await;
    wait_for(&mut b_rx, "Alice at Bob", |e| {
        matches!(e, Event::PeerOnline { .. })
    })
    .await;

    let src = _db.path().join("huge.bin");
    std::fs::write(&src, vec![7u8; 32 * 1024 * 1024]).unwrap();
    b.send_file(Some(a.identity.id.clone()), &src)
        .await
        .unwrap();
    // Let a few chunks land, then kill the sender.
    wait_for(&mut a_rx, "first progress at Alice", |e| {
        matches!(
            e,
            Event::Transfer {
                direction: TransferDirection::Receiving,
                ..
            }
        )
    })
    .await;
    b.shutdown();

    let ev = wait_for(&mut a_rx, "failure at Alice", |e| {
        matches!(e, Event::TransferFailed { .. } | Event::Message { .. })
    })
    .await;
    assert!(matches!(ev, Event::TransferFailed { .. }), "got {ev:?}");
    let leftovers: Vec<_> = std::fs::read_dir(da.path().join("files"))
        .unwrap()
        .flatten()
        .collect();
    assert!(leftovers.is_empty(), "left {:?}", leftovers);
    assert!(a.messages().is_empty());
}

/// A scan probe learns who is at an address without the probed node ever
/// seeing a peer come online.
#[tokio::test]
async fn probe_reads_hello_without_registering() {
    let (a, mut a_rx, _da) = spawn("Dapur", false, vec![]).await;
    let (b, _b_rx, _db) = spawn("Kantor", true, vec![]).await;
    let addr: std::net::SocketAddr = format!("127.0.0.1:{}", a.port()).parse().unwrap();

    let found = lanchat_net::scan::probe(addr, &b.identity.id)
        .await
        .unwrap()
        .expect("a is not b");
    assert_eq!(found.id, a.identity.id);
    assert_eq!(found.name, "Dapur");
    assert_eq!(found.addr, addr.to_string());
    assert!(!found.relay);
    assert_eq!(found.via, FoundVia::Scan);
    assert!(!found.connected);

    // Probing our own address is reported as "nobody", not as a peer.
    assert!(lanchat_net::scan::probe(addr, &a.identity.id)
        .await
        .unwrap()
        .is_none());

    // The probed node saw nothing: no peer event, no connection.
    let quiet = timeout(Duration::from_millis(500), a_rx.recv()).await;
    assert!(quiet.is_err(), "unexpected event: {quiet:?}");
    assert!(a.peers().is_empty());

    // The relay flag travels with the hello too.
    let addr_b: std::net::SocketAddr = format!("127.0.0.1:{}", b.port()).parse().unwrap();
    let found_b = lanchat_net::scan::probe(addr_b, &a.identity.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found_b.name, "Kantor");
    assert!(found_b.relay);

    // A silent port is an error, not a hang.
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let silent = l.local_addr().unwrap();
    let t0 = std::time::Instant::now();
    assert!(lanchat_net::scan::probe(silent, &a.identity.id)
        .await
        .is_err());
    assert!(t0.elapsed() < Duration::from_secs(5));
}

#[test]
fn subnets_are_deduplicated_and_filtered() {
    use std::net::Ipv4Addr;
    let s = lanchat_net::scan::subnets_of(&[
        Ipv4Addr::new(192, 168, 1, 23),
        Ipv4Addr::new(192, 168, 1, 99),
        Ipv4Addr::new(10, 0, 5, 7),
        Ipv4Addr::new(127, 0, 0, 1),
        Ipv4Addr::new(169, 254, 3, 3),
    ]);
    assert_eq!(s, vec![[10, 0, 5], [192, 168, 1]]);
}
