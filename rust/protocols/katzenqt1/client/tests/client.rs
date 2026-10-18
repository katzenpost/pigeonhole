//! Two clients over the in-memory Pigeonhole, in real time, with their state synced to disk.

use std::time::{Duration, Instant};

use pigeonhole_katzenqt1_client::{Client, ClientError, Config};
use pigeonhole_transport::sim::World;

fn tmp() -> tempfile::TempDir {
    tempfile::TempDir::new_in(
        std::env::var("TMPDIR")
            .unwrap_or_else(|_| format!("{}/scratch/tmp", std::env::var("HOME").unwrap())),
    )
    .unwrap()
}

async fn open(w: &World, dir: &std::path::Path, name: &str) -> Client {
    Client::open(
        Config {
            state_dir: dir.into(),
            name: name.into(),
            epoch_poll: Duration::from_secs(1),
            no_sync: false,
        },
        w.clone(),
    )
    .await
    .unwrap()
}

async fn wait_for<F: std::future::Future<Output = bool>>(what: &str, mut ok: impl FnMut() -> F) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !ok().await {
        assert!(Instant::now() < deadline, "waited a minute for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn has(c: &Client, id: &str, text: &str) -> bool {
    c.lines(id, None)
        .await
        .unwrap()
        .iter()
        .any(|l| l.line.notice.is_none() && l.line.text == text)
}

#[tokio::test(flavor = "multi_thread")]
async fn client() {
    let w = World::default();
    let (alice_dir, bob_dir) = (tmp(), tmp());
    let alice = open(&w, alice_dir.path(), "alice").await;
    let bob = open(&w, bob_dir.path(), "bob").await;
    let mut events = alice.subscribe();
    let g = alice.create("g").await.unwrap();
    assert_eq!(alice.create("g").await, Err(ClientError::NameTaken));
    assert_eq!(alice.send("nope", "x").await, Err(ClientError::NoSuchChat));
    let bg = bob.join("g").await.unwrap();
    let mut code = String::new();
    wait_for("a voucher code", || {
        let (bob, bg) = (&bob, &bg);
        async move { !bob.chat(bg).await.unwrap().code.is_empty() }
    })
    .await;
    code.push_str(&bob.chat(&bg).await.unwrap().code);
    wait_for("alice's group", || async {
        alice.chat(&g).await.unwrap().joined
    })
    .await;
    assert_eq!(
        alice.invite(&g, "abc!").await,
        Err(ClientError::NotAVoucher)
    );
    alice.invite(&g, &code).await.unwrap();
    wait_for("bob to join", || async {
        bob.chat(&bg).await.unwrap().joined
    })
    .await;
    bob.send(&bg, "hi alice").await.unwrap();
    alice.send(&g, "hi bob").await.unwrap();
    wait_for("each to read the other", || async {
        has(&alice, &g, "hi alice").await && has(&bob, &bg, "hi bob").await
    })
    .await;
    assert_eq!(events.try_recv().unwrap().chat, g);
    let bob_key = alice.member_named(&g, "bob").await.unwrap();
    assert!(!bob_key.is_empty());
    let me = alice.chat(&g).await.unwrap().me;
    assert_eq!(alice.remove(&g, me).await, Err(ClientError::RemoveSelf));
    assert_eq!(
        alice.remove(&g, b"nobody".to_vec()).await,
        Err(ClientError::NoSuchMember)
    );
    // Alice closes, and opens again where she was.
    alice.close().await;
    assert_eq!(alice.send(&g, "closed").await, Err(ClientError::Closed));
    let alice = open(&w, alice_dir.path(), "alice").await;
    assert!(
        has(&alice, &g, "hi alice").await,
        "alice forgets what she read"
    );
    alice.send(&g, "back again").await.unwrap();
    bob.send(&bg, "read you").await.unwrap();
    wait_for("the conversation to carry on", || async {
        has(&bob, &bg, "back again").await && has(&alice, &g, "read you").await
    })
    .await;
    wait_for("bob to acknowledge", || async {
        alice
            .lines(&g, None)
            .await
            .unwrap()
            .iter()
            .any(|l| l.line.text == "hi bob" && l.mine && l.acked)
    })
    .await;
    assert!(w.state().refused.is_empty());
    alice.close().await;
    bob.close().await;
}
