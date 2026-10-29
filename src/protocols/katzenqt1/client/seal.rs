//! Sealing what the store keeps under a key of 32 bytes: XChaCha20-Poly1305 with a random nonce,
//! bound by its associated data to where the sealed bytes are kept, so that they open nowhere
//! else.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::RngCore;
use zeroize::Zeroizing;

const FORMAT: u8 = 1;
const NONCE: usize = 24;

/// The bytes did not open: another key sealed them, or they were sealed for another place, or
/// they were changed.
#[derive(Debug)]
pub struct Unopened;

fn cipher(key: &[u8]) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new_from_slice(key).expect("a key of 32 bytes")
}

/// Where a value is kept, as associated data: its table, then each part of its key with its
/// length before it.
pub fn place(table: &str, parts: &[&[u8]]) -> Vec<u8> {
    let mut out = table.as_bytes().to_vec();
    for p in parts {
        out.extend((p.len() as u32).to_be_bytes());
        out.extend(*p);
    }
    out
}

pub fn seal(key: &[u8], aad: &[u8], plain: &[u8]) -> Vec<u8> {
    let mut nonce = [0u8; NONCE];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let ct = cipher(key)
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad })
        .expect("sealing does not fail");
    let mut out = Vec::with_capacity(1 + NONCE + ct.len());
    out.push(FORMAT);
    out.extend(nonce);
    out.extend(ct);
    out
}

pub fn open(key: &[u8], aad: &[u8], sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, Unopened> {
    if sealed.len() < 1 + NONCE || sealed[0] != FORMAT {
        return Err(Unopened);
    }
    let (nonce, ct) = sealed[1..].split_at(NONCE);
    cipher(key)
        .decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad })
        .map(Zeroizing::new)
        .map_err(|_| Unopened)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_only_where_sealed() {
        let k = [7u8; 32];
        let s = seal(&k, &place("lines", &[b"g", &0u64.to_be_bytes()]), b"hello");
        assert_eq!(
            &*open(&k, &place("lines", &[b"g", &0u64.to_be_bytes()]), &s).unwrap(),
            b"hello"
        );
        assert!(open(&k, &place("lines", &[b"g", &1u64.to_be_bytes()]), &s).is_err());
        assert!(open(&[8u8; 32], &place("lines", &[b"g", &0u64.to_be_bytes()]), &s).is_err());
        let mut t = s.clone();
        *t.last_mut().unwrap() ^= 1;
        assert!(open(&k, &place("lines", &[b"g", &0u64.to_be_bytes()]), &t).is_err());
    }

    #[test]
    fn parts_cannot_run_together() {
        assert_ne!(place("t", &[b"ab", b"c"]), place("t", &[b"a", b"bc"]));
    }
}
