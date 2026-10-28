//! Anonymous sealed boxes, as hpqc's `protocols/sealedbox` makes them:
//!
//! ```text
//! ss          = DH(eph, recipient)
//! key, nonce  = HKDF-BLAKE2b-256(ss, info = label || u16be(len name) || name || ephPub || recipientPub)
//! box         = ephPub || secretbox(key, nonce, plaintext)
//! ```

use blake2::Blake2b;
use blake2::digest::consts::U32;
use crypto_secretbox::aead::{Aead, KeyInit};
use crypto_secretbox::{Key, Nonce, XSalsa20Poly1305};
use zeroize::Zeroizing;

use crate::services::Error;
use crate::services::nike::{self, PrivateKey, PublicKey};

const LABEL: &[u8] = b"hpqc-nike-sealedbox-v1";
const KEY_SIZE: usize = 32;
const NONCE_SIZE: usize = 24;
const TAG_SIZE: usize = 16;

pub(crate) type Hkdf = hkdf::SimpleHkdf<Blake2b<U32>>;

fn derive(priv_key: &PrivateKey, pub_key: &PublicKey, eph_pub: &[u8], recipient_pub: &[u8]) -> Result<(XSalsa20Poly1305, Nonce), Error> {
    let ss = priv_key.derive_secret(pub_key)?;
    let name = nike::SCHEME.as_bytes();
    let mut info = Vec::with_capacity(LABEL.len() + 2 + name.len() + eph_pub.len() + recipient_pub.len());
    info.extend_from_slice(LABEL);
    info.extend_from_slice(&(name.len() as u16).to_be_bytes());
    info.extend_from_slice(name);
    info.extend_from_slice(eph_pub);
    info.extend_from_slice(recipient_pub);
    let mut okm = Zeroizing::new([0u8; KEY_SIZE + NONCE_SIZE]);
    Hkdf::new(None, &ss).expand(&info, &mut okm[..]).map_err(|_| Error::Crypto)?;
    let cipher = XSalsa20Poly1305::new(Key::from_slice(&okm[..KEY_SIZE]));
    Ok((cipher, *Nonce::from_slice(&okm[KEY_SIZE..])))
}

/// Seals `plaintext` to `recipient`.
pub fn seal(recipient: &PublicKey, plaintext: &[u8]) -> Result<Vec<u8>, Error> {
    let eph = PrivateKey::generate();
    let eph_pub = eph.public_key().to_bytes();
    let (cipher, nonce) = derive(&eph, recipient, &eph_pub, &recipient.to_bytes())?;
    let sealed = cipher.encrypt(&nonce, plaintext).map_err(|_| Error::Crypto)?;
    let mut out = eph_pub;
    out.extend_from_slice(&sealed);
    Ok(out)
}

/// Opens a box sealed to `public`, the public key of `private`.
pub fn open(private: &PrivateKey, public: &PublicKey, ciphertext: &[u8]) -> Result<Vec<u8>, Error> {
    if ciphertext.len() < nike::PUBLIC_KEY_SIZE + TAG_SIZE {
        return Err(Error::Crypto);
    }
    let (eph_bytes, sealed) = ciphertext.split_at(nike::PUBLIC_KEY_SIZE);
    let eph = PublicKey::from_bytes(eph_bytes).map_err(|_| Error::Crypto)?;
    let (cipher, nonce) = derive(private, &eph, eph_bytes, &public.to_bytes()).map_err(|_| Error::Crypto)?;
    cipher.decrypt(&nonce, sealed).map_err(|_| Error::Crypto)
}
