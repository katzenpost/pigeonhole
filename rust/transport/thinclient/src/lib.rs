//! A Pigeonhole transport over the katzenpost client daemon, through its thin client.

use std::collections::BTreeMap;
use std::sync::Arc;

use katzenpost_thin_client::error::ThinClientError;
use katzenpost_thin_client::transport::{DialConfig, TcpDialConfig, UnixDialConfig};
use katzenpost_thin_client::{Config, ThinClient};
use pigeonhole_transport::{Backend, Error, Inducted, Keypair, Minted, Opened, Read, Result};
use rand::RngCore;
use serde_cbor::Value;

pub struct ThinClientBackend {
    pub client: Arc<ThinClient>,
}

fn err(e: ThinClientError) -> Error {
    match e {
        ThinClientError::BoxAlreadyExists => Error::BoxAlreadyExists,
        e => Error::Other(e.to_string()),
    }
}

/// How to reach the daemon at `address`: `@name` is an abstract unix socket, a path a unix
/// socket, and anything else `host:port` over TCP.
pub fn dial_config(address: &str) -> DialConfig {
    if address.starts_with('@') || address.starts_with('/') {
        DialConfig {
            unix: Some(UnixDialConfig {
                address: address.into(),
            }),
            tcp: None,
        }
    } else {
        DialConfig {
            unix: None,
            tcp: Some(TcpDialConfig {
                address: address.into(),
                network: None,
            }),
        }
    }
}

impl ThinClientBackend {
    /// Connect to the daemon at `address`.
    pub async fn dial(
        address: &str,
    ) -> std::result::Result<ThinClientBackend, Box<dyn std::error::Error>> {
        let config = Config {
            dial: dial_config(address),
            on_connection_status: None,
            on_new_pki_document: None,
            on_message_sent: None,
            on_message_reply: None,
            on_daemon_disconnected: None,
        };
        Ok(ThinClientBackend {
            client: ThinClient::new(config).await?,
        })
    }

    pub async fn close(&self) {
        self.client.stop().await;
    }
}

/// The epoch most storage replicas are in: each publishes envelope keys up to the epoch after its
/// own. A replica's descriptor comes as a map, or as its CBOR.
pub fn replica_epoch_of(doc: &BTreeMap<Value, Value>) -> Option<u64> {
    let Some(Value::Array(replicas)) = doc.get(&Value::Text("StorageReplicas".into())) else {
        return None;
    };
    let mut votes: BTreeMap<u64, usize> = BTreeMap::new();
    for r in replicas {
        let desc = match r {
            Value::Map(m) => m.clone(),
            Value::Bytes(b) => match serde_cbor::from_slice::<Value>(b) {
                Ok(Value::Map(m)) => m,
                _ => continue,
            },
            _ => continue,
        };
        let Some(Value::Map(keys)) = desc.get(&Value::Text("EnvelopeKeys".into())) else {
            continue;
        };
        let top = keys
            .keys()
            .filter_map(|k| match k {
                Value::Integer(i) => u64::try_from(*i).ok(),
                _ => None,
            })
            .max();
        if let Some(top) = top.filter(|&t| t > 0) {
            *votes.entry(top - 1).or_default() += 1;
        }
    }
    let best = votes.values().max()?;
    votes.iter().find(|(_, n)| *n == best).map(|(e, _)| *e)
}

impl Backend for ThinClientBackend {
    async fn new_keypair(&self) -> Result<Keypair> {
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);
        let k = self.client.new_keypair(&seed).await.map_err(err)?;
        Ok(Keypair {
            write_cap: k.write_cap,
            read_cap: k.read_cap,
            first: k.first_message_index,
        })
    }

    async fn write_box(&self, write_cap: &[u8], index: &[u8], plaintext: &[u8]) -> Result<()> {
        let e = self
            .client
            .encrypt_write(plaintext, write_cap, index)
            .await
            .map_err(err)?;
        self.client
            .start_resending_encrypted_message_return_box_exists(
                None,
                Some(write_cap),
                None,
                None,
                &e.envelope_descriptor,
                &e.message_ciphertext,
                &e.envelope_hash,
            )
            .await
            .map_err(err)?;
        Ok(())
    }

    async fn read_box(&self, read_cap: &[u8], index: &[u8]) -> Result<Read> {
        let e = self
            .client
            .encrypt_read(read_cap, index)
            .await
            .map_err(err)?;
        let r = self
            .client
            .start_resending_encrypted_message_no_retry(
                Some(read_cap),
                None,
                Some(index),
                None,
                &e.envelope_descriptor,
                &e.message_ciphertext,
                &e.envelope_hash,
            )
            .await;
        match r {
            Err(ThinClientError::BoxNotFound) => Ok(Read::NotFound),
            Err(ThinClientError::Tombstone) => Ok(Read::Tombstone),
            Err(e) => Err(err(e)),
            Ok(r) if r.plaintext.is_empty() => Ok(Read::Tombstone),
            Ok(r) => Ok(Read::Data(r.plaintext)),
        }
    }

    async fn replica_epoch(&self) -> Result<u64> {
        let doc = self.client.pki_document().await.map_err(err)?;
        replica_epoch_of(&doc).ok_or_else(|| Error::Other("no storage replica epochs".into()))
    }

    async fn max_plaintext(&self) -> Result<usize> {
        Ok(self
            .client
            .pigeonhole_geometry()
            .max_plaintext_payload_length)
    }

    async fn voucher_mint(&self, write_cap: &[u8], name: &str) -> Result<Minted> {
        let r = self
            .client
            .voucher_mint(write_cap, name)
            .await
            .map_err(err)?;
        Ok(Minted {
            voucher: r.voucher,
            payload: r.voucher_payload,
            write_cap: r.voucher_write_cap,
            read_cap: r.voucher_read_cap,
            secret_key: r.voucher_secret_key,
        })
    }

    async fn voucher_derive_stream(&self, voucher: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        let r = self
            .client
            .voucher_derive_stream(voucher)
            .await
            .map_err(err)?;
        Ok((r.voucher_write_cap, r.voucher_read_cap))
    }

    async fn voucher_induct(
        &self,
        voucher: &[u8],
        payload: &[u8],
        who_reply: &[u8],
    ) -> Result<Inducted> {
        let r = self
            .client
            .voucher_induct(voucher, payload, who_reply)
            .await
            .map_err(err)?;
        Ok(Inducted {
            name: r.display_name,
            mutated_read_cap: r.mutated_message_read_cap,
            sealed_reply: r.sealed_reply,
        })
    }

    async fn voucher_open(
        &self,
        secret_key: &[u8],
        sealed_reply: &[u8],
        write_cap: &[u8],
    ) -> Result<Opened> {
        let r = self
            .client
            .voucher_open(secret_key, sealed_reply, write_cap)
            .await
            .map_err(err)?;
        Ok(Opened {
            who_reply: r.who_reply,
            mutated_write_cap: r.mutated_message_write_cap,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replica(epochs: &[i128]) -> Value {
        Value::Map(
            [(
                Value::Text("EnvelopeKeys".into()),
                Value::Map(
                    epochs
                        .iter()
                        .map(|&e| (Value::Integer(e), Value::Bytes(b"k".to_vec())))
                        .collect(),
                ),
            )]
            .into(),
        )
    }

    #[test]
    fn replica_epoch() {
        let doc: BTreeMap<Value, Value> = [(
            Value::Text("StorageReplicas".into()),
            Value::Array(vec![
                replica(&[4, 5, 6]),
                Value::Bytes(serde_cbor::to_vec(&replica(&[5, 6])).unwrap()),
                replica(&[7, 8]),
                Value::Bytes(b"junk".to_vec()),
                Value::Map(BTreeMap::new()),
            ]),
        )]
        .into();
        assert_eq!(replica_epoch_of(&doc), Some(5));
        assert_eq!(replica_epoch_of(&BTreeMap::new()), None);
    }

    /// Over the client daemon at `$DAEMON`: the same bytes written again to a box succeed, as a
    /// write tried again after a crash must, and other bytes are refused.
    #[tokio::test]
    async fn live_rewrite() {
        let Ok(daemon) = std::env::var("DAEMON") else {
            return;
        };
        let b = ThinClientBackend::dial(&daemon).await.unwrap();
        let kp = b.new_keypair().await.unwrap();
        let bytes = b"the same bytes, twice";
        for _ in 0..2 {
            b.write_box(&kp.write_cap, &kp.first, bytes).await.unwrap();
        }
        assert!(matches!(
            b.write_box(&kp.write_cap, &kp.first, b"other bytes").await,
            Err(Error::BoxAlreadyExists)
        ));
        assert_eq!(
            b.read_box(&kp.read_cap, &kp.first).await.unwrap(),
            Read::Data(bytes.to_vec())
        );
        b.close().await;
    }

    #[test]
    fn dial() {
        assert!(dial_config("@katzenpost").unix.is_some());
        assert!(dial_config("localhost:64331").tcp.is_some());
    }
}
