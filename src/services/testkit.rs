//! A voucher router and a service directory on [`crate::transport::sim`], enough to run
//! services end to end in tests.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::services::conn::Conn;
use crate::transport::sim::{Handler, World};
use crate::services::nike::{self, Keypair};
use crate::services::wire::*;
use crate::services::{auth, induct};

/// The poll interval the test router uses.
pub const POLL: Duration = Duration::from_millis(10);

/// What a plugin with `key` publishes in the PKI.
pub fn params(key: &Keypair) -> BTreeMap<String, serde_cbor::Value> {
    BTreeMap::from([
        (PARAM_NIKE_SCHEME.to_string(), serde_cbor::Value::Text(nike::SCHEME.into())),
        (PARAM_PUBLIC_KEY.to_string(), serde_cbor::Value::Bytes(key.public.to_bytes())),
    ])
}

/// Adds a router at `node`.
pub fn add_router(net: &World, node: [u8; 32]) {
    let key = Keypair::generate();
    let links: Arc<Mutex<HashMap<ServiceId, Arc<Conn<World>>>>> = Arc::default();
    let net2 = net.clone();
    let k = key.clone();
    let h: Handler = Arc::new(move |payload: Vec<u8>| {
        let (net, key, links) = (net2.clone(), k.clone(), links.clone());
        Box::pin(async move {
            let status = match RouterRequest::decode(&payload) {
                Ok(RouterRequest::Register(r)) => {
                    if auth::verify_register(&key, &r).is_err() {
                        Status::BadMac
                    } else {
                        let id = ServiceId::from_public_key(&r.public_key);
                        tokio::spawn(async move {
                            if let Ok(conn) = induct(Arc::new(net), &r.voucher, "router", POLL).await {
                                links.lock().unwrap().insert(id, Arc::new(conn));
                            }
                        });
                        Status::Ok
                    }
                }
                Ok(RouterRequest::Route(r)) => {
                    let link = links.lock().unwrap().get(&r.service_id).cloned();
                    match link {
                        Some(c) => match c.send(&encode_link_message(&r.ciphertext)).await {
                            Ok(()) => Status::Ok,
                            Err(_) => Status::InternalError,
                        },
                        None => Status::UnknownService,
                    }
                }
                Err(_) => Status::InvalidRequest,
            };
            encode_router_reply(status)
        })
    });
    net.add_service(ROUTER_CAPABILITY, node, ROUTER_CAPABILITY, params(&key), h);
}

/// Adds a directory at `node`.
pub fn add_directory(net: &World, node: [u8; 32]) {
    let key = Keypair::generate();
    let descs: Arc<Mutex<HashMap<ServiceId, (u64, ServiceDescriptor)>>> = Arc::default();
    let k = key.clone();
    let h: Handler = Arc::new(move |payload: Vec<u8>| {
        let (key, descs) = (k.clone(), descs.clone());
        Box::pin(async move {
            let mut reply = DirectoryReply { status: Status::Ok as u8, descriptor: None, read_cap: vec![], start_index: vec![] };
            match DirectoryRequest::decode(&payload) {
                Ok(DirectoryRequest::Advertise(a)) => {
                    let mut d = descs.lock().unwrap();
                    let id = a.descriptor.id();
                    if auth::verify_advertise(&key, &a).is_err() {
                        reply.status = Status::BadMac as u8;
                    } else if d.get(&id).is_some_and(|(seq, _)| *seq >= a.seq) {
                        reply.status = Status::Stale as u8;
                    } else {
                        d.insert(id, (a.seq, a.descriptor));
                    }
                }
                Ok(DirectoryRequest::Lookup(id)) => match descs.lock().unwrap().get(&id) {
                    Some((_, d)) => reply.descriptor = Some(d.clone()),
                    None => reply.status = Status::UnknownService as u8,
                },
                _ => reply.status = Status::InvalidRequest as u8,
            }
            reply.encode()
        })
    });
    net.add_service(DIRECTORY_CAPABILITY, node, DIRECTORY_CAPABILITY, params(&key), h);
}

/// A network with two routers and a directory.
pub fn world() -> World {
    let net = World::default();
    add_router(&net, [0xa0; 32]);
    add_router(&net, [0xa1; 32]);
    add_directory(&net, [0xdd; 32]);
    net
}

/// Waits until the directory lists the service with `n` routers.
pub async fn lookup_linked(net: &World, id: ServiceId, n: usize) -> ServiceDescriptor {
    let dir = crate::services::Directory::all(Arc::new(net.clone()), None, POLL).await.unwrap().remove(0);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(d) = dir.lookup(id).await
            && d.routers.len() == n
        {
            return d;
        }
        assert!(tokio::time::Instant::now() < deadline, "service never linked with {n} routers");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
