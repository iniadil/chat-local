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
