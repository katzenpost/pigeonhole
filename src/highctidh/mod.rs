//! CTIDH-1024, a post-quantum non-interactive key exchange, bound from the
//! highctidh C library. Keys serialize as hpqc's `ctidh1024` scheme does,
//! so they interoperate with Go katzenpost code.

use std::fmt;

use zeroize::{Zeroize, ZeroizeOnDrop};

/// Size of a serialized public key.
pub const PUBLIC_KEY_SIZE: usize = 128;
/// Size of a serialized private key.
pub const PRIVATE_KEY_SIZE: usize = 130;
/// Size of a shared secret.
pub const SHARED_SECRET_SIZE: usize = PUBLIC_KEY_SIZE;

const LIMBS: usize = PUBLIC_KEY_SIZE / 8;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct RawPublicKey {
    limbs: [u64; LIMBS],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RawPrivateKey {
    e: [i8; PRIVATE_KEY_SIZE],
}

unsafe extern "C" {
    fn highctidh_1024_csidh_private(private: *mut RawPrivateKey);
    fn highctidh_1024_csidh_private_valid(private: *const RawPrivateKey) -> bool;
    fn highctidh_1024_csidh(out: *mut RawPublicKey, input: *const RawPublicKey, private: *const RawPrivateKey) -> bool;
    fn highctidh_1024_validate(input: *const RawPublicKey) -> bool;
}

/// Errors from decoding keys or computing the group action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The key is not the right length.
    Length,
    /// The bytes do not encode a valid key.
    Invalid,
    /// The group action failed.
    GroupAction,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::Length => "ctidh1024: wrong key length",
            Error::Invalid => "ctidh1024: invalid key",
            Error::GroupAction => "ctidh1024: group action failed",
        })
    }
}

impl std::error::Error for Error {}

/// A CTIDH-1024 public key.
#[derive(Clone, PartialEq, Eq)]
pub struct PublicKey(RawPublicKey);

impl PublicKey {
    /// Decodes and validates a public key.
    pub fn from_bytes(b: &[u8]) -> Result<Self, Error> {
        if b.len() != PUBLIC_KEY_SIZE {
            return Err(Error::Length);
        }
        let mut raw = RawPublicKey { limbs: [0; LIMBS] };
        for (limb, chunk) in raw.limbs.iter_mut().zip(b.as_chunks::<8>().0) {
            *limb = u64::from_le_bytes(*chunk);
        }
        if raw.limbs == [0; LIMBS] || !unsafe { highctidh_1024_validate(&raw) } {
            return Err(Error::Invalid);
        }
        Ok(PublicKey(raw))
    }

    /// Encodes the public key.
    pub fn to_bytes(&self) -> [u8; PUBLIC_KEY_SIZE] {
        let mut out = [0u8; PUBLIC_KEY_SIZE];
        for (chunk, limb) in out.as_chunks_mut::<8>().0.iter_mut().zip(self.0.limbs) {
            *chunk = limb.to_le_bytes();
        }
        out
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey(")?;
        for b in &self.to_bytes()[..8] {
            write!(f, "{b:02x}")?;
        }
        write!(f, "…)")
    }
}

/// A CTIDH-1024 private key.
#[derive(Clone)]
pub struct PrivateKey(RawPrivateKey);

impl Zeroize for PrivateKey {
    fn zeroize(&mut self) {
        self.0.e.zeroize();
    }
}

impl Drop for PrivateKey {
    fn drop(&mut self) {
        self.zeroize();
    }
}

impl ZeroizeOnDrop for PrivateKey {}

impl PrivateKey {
    /// Generates a private key from the operating system's randomness.
    pub fn generate() -> Self {
        let mut raw = RawPrivateKey { e: [0; PRIVATE_KEY_SIZE] };
        unsafe { highctidh_1024_csidh_private(&mut raw) };
        PrivateKey(raw)
    }

    /// Decodes and validates a private key.
    pub fn from_bytes(b: &[u8]) -> Result<Self, Error> {
        if b.len() != PRIVATE_KEY_SIZE {
            return Err(Error::Length);
        }
        let mut raw = RawPrivateKey { e: [0; PRIVATE_KEY_SIZE] };
        for (e, &v) in raw.e.iter_mut().zip(b) {
            *e = v as i8;
        }
        let key = PrivateKey(raw);
        if !unsafe { highctidh_1024_csidh_private_valid(&key.0) } {
            return Err(Error::Invalid);
        }
        Ok(key)
    }

    /// Encodes the private key.
    pub fn to_bytes(&self) -> [u8; PRIVATE_KEY_SIZE] {
        self.0.e.map(|v| v as u8)
    }

    /// Returns the public key.
    pub fn public_key(&self) -> PublicKey {
        let base = RawPublicKey { limbs: [0; LIMBS] };
        PublicKey(self.act(&base).expect("ctidh1024: group action on the base curve"))
    }

    /// Computes the shared secret with a peer's public key.
    pub fn derive_secret(&self, peer: &PublicKey) -> Result<[u8; SHARED_SECRET_SIZE], Error> {
        Ok(PublicKey(self.act(&peer.0)?).to_bytes())
    }

    fn act(&self, input: &RawPublicKey) -> Result<RawPublicKey, Error> {
        let mut out = RawPublicKey { limbs: [0; LIMBS] };
        if unsafe { highctidh_1024_csidh(&mut out, input, &self.0) } {
            Ok(out)
        } else {
            Err(Error::GroupAction)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_secrets_agree() {
        let a = PrivateKey::generate();
        let b = PrivateKey::generate();
        let ab = a.derive_secret(&b.public_key()).unwrap();
        let ba = b.derive_secret(&a.public_key()).unwrap();
        assert_eq!(ab, ba);
        assert_ne!(ab, [0; SHARED_SECRET_SIZE]);
    }

    #[test]
    fn keys_round_trip() {
        let a = PrivateKey::generate();
        let pub_a = a.public_key();
        let a2 = PrivateKey::from_bytes(&a.to_bytes()).unwrap();
        assert_eq!(a2.public_key(), pub_a);
        assert_eq!(PublicKey::from_bytes(&pub_a.to_bytes()).unwrap(), pub_a);
    }

    #[test]
    fn rejects_bad_public_keys() {
        assert_eq!(PublicKey::from_bytes(&[0; PUBLIC_KEY_SIZE]), Err(Error::Invalid));
        assert_eq!(PublicKey::from_bytes(&[0; 3]), Err(Error::Length));
        let mut b = PrivateKey::generate().public_key().to_bytes();
        b[0] ^= 1;
        assert_eq!(PublicKey::from_bytes(&b), Err(Error::Invalid));
    }
}
