use std::sync::Arc;
use std::time::Duration;

use pigeonhole_protocols::services::testkit::*;
use pigeonhole_protocols::transport::sim::World;
use pigeonhole_protocols::services::{Config, Keypair, Service, dial, induct, mint};

fn config(dir: &std::path::Path) -> Config {
    let mut cfg = Config::new(Keypair::load_or_generate(dir, "service").unwrap(), dir);
    cfg.display_name = "echo".into();
    cfg.poll_interval = POLL;
    cfg
}

fn echo(net: &World, cfg: Config) -> Service<World> {
    let svc = Service::new(cfg, Arc::new(net.clone())).unwrap();
    let s = svc.clone();
    tokio::spawn(async move {
        s.run(|session| async move {
            while let Ok(msg) = session.recv().await {
                let mut reply = b"echo: ".to_vec();
                reply.extend_from_slice(&msg);
                if session.send(&reply).await.is_err() {
                    return;
                }
            }
        })
        .await
        .unwrap();
    });
    svc
}

async fn within<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(30), f).await.expect("timed out")
}

#[tokio::test(flavor = "multi_thread")]
async fn handshake_opens_a_two_way_connection() {
    let net = Arc::new(World::default());
    let pending = mint(&*net, "client").await.unwrap();
    let restored = pigeonhole_protocols::services::Pending::decode(&pending.encode()).unwrap();
    assert_eq!(restored, pending);
    let n = net.clone();
    let v = pending.voucher.clone();
    let inductor = tokio::spawn(async move { induct(n, &v, "service", POLL).await.unwrap() });
    let joiner = within(restored.wait(net.clone(), POLL)).await.unwrap();
    let inductor = inductor.await.unwrap();

    joiner.send(b"hello service").await.unwrap();
    assert_eq!(within(inductor.recv()).await.unwrap(), b"hello service");
    inductor.send(b"hello client").await.unwrap();
    assert_eq!(within(joiner.recv()).await.unwrap(), b"hello client");
    for i in 1..=5u8 {
        joiner.send(&[i]).await.unwrap();
    }
    for i in 1..=5u8 {
        assert_eq!(within(inductor.recv()).await.unwrap(), [i], "messages arrive in order");
    }
    assert!(induct(net.clone(), &pending.voucher, "again", POLL).await.is_err(), "a voucher inducts once");
}

#[tokio::test(flavor = "multi_thread")]
async fn service_end_to_end() {
    let net = world();
    let dir = tempfile::tempdir().unwrap();
    let svc = echo(&net, config(dir.path()));
    let d = lookup_linked(&net, svc.id(), 2).await;
    let conn = within(dial(Arc::new(net.clone()), &d, "alice", POLL)).await.unwrap();
    for m in ["hello", "again"] {
        conn.send(m.as_bytes()).await.unwrap();
        assert_eq!(within(conn.recv()).await.unwrap(), format!("echo: {m}").as_bytes());
    }
    svc.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn offered_voucher_opens_a_session() {
    let net = world();
    let dir = tempfile::tempdir().unwrap();
    let svc = echo(&net, config(dir.path()));
    lookup_linked(&net, svc.id(), 2).await;
    let voucher = within(svc.offer()).await.unwrap();
    assert_eq!(svc.offers(), vec![voucher.clone()]);

    let conn = within(induct(Arc::new(net.clone()), &voucher, "bob", POLL)).await.unwrap();
    conn.send(b"hi").await.unwrap();
    assert_eq!(within(conn.recv()).await.unwrap(), b"echo: hi");
    assert!(svc.offers().is_empty(), "a used offer is forgotten");
    svc.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn service_resumes_after_restart() {
    let net = world();
    let dir = tempfile::tempdir().unwrap();
    let svc = echo(&net, config(dir.path()));
    let d = lookup_linked(&net, svc.id(), 2).await;
    let conn = within(dial(Arc::new(net.clone()), &d, "alice", POLL)).await.unwrap();
    conn.send(b"one").await.unwrap();
    assert_eq!(within(conn.recv()).await.unwrap(), b"echo: one");
    let offered = within(svc.offer()).await.unwrap();
    svc.close().await;

    // The links, the session and the offer come back without registering
    // or dialling again.
    let writes = net.state().writes;
    let svc = echo(&net, config(dir.path()));
    conn.send(b"two").await.unwrap();
    assert_eq!(within(conn.recv()).await.unwrap(), b"echo: two");
    let late = within(induct(Arc::new(net.clone()), &offered, "late", POLL)).await.unwrap();
    late.send(b"three").await.unwrap();
    assert_eq!(within(late.recv()).await.unwrap(), b"echo: three");
    assert!(net.state().writes > writes);
    assert_eq!(svc.descriptor().routers.len(), 2);
    svc.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dial_falls_back_to_another_router() {
    let net = world();
    let dir = tempfile::tempdir().unwrap();
    let svc = echo(&net, config(dir.path()));
    let mut d = lookup_linked(&net, svc.id(), 2).await;
    // The first "router" is the directory, which refuses a router request.
    d.routers.insert(0, pigeonhole_protocols::services::wire::RouterRef { node_id_hash: [0xdd; 32], endpoint: "service_directory".into() });
    let conn = within(dial(Arc::new(net.clone()), &d, "carol", POLL)).await.unwrap();
    conn.send(b"x").await.unwrap();
    assert_eq!(within(conn.recv()).await.unwrap(), b"echo: x");
    svc.close().await;
}
