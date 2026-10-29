//! The X25519 and CTIDH-1024 hybrid NIKE that services, routers and
//! directories key themselves with. Keys and shared secrets are laid out as
//! hpqc's `hybrid.X25519CTIDH1024`: the X25519 part first, then CTIDH.

use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use rand::rngs::OsRng;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::services::Error;

/// The scheme's name, as hpqc spells it.
pub const SCHEME: &str = "X25519-CTIDH1024";

const X25519_SIZE: usize = 32;
/// Size of a serialized public key.
pub const PUBLIC_KEY_SIZE: usize = X25519_SIZE + crate::highctidh::PUBLIC_KEY_SIZE;
/// Size of a serialized private key.
pub const PRIVATE_KEY_SIZE: usize = X25519_SIZE + crate::highctidh::PRIVATE_KEY_SIZE;

/// A hybrid public key.
#[derive(Clone, PartialEq, Eq)]
pub struct PublicKey {
    x25519: x25519_dalek::PublicKey,
    ctidh: crate::highctidh::PublicKey,
}

impl PublicKey {
    /// Decodes a public key, validating its CTIDH part.
    pub fn from_bytes(b: &[u8]) -> Result<Self, Error> {
        if b.len() != PUBLIC_KEY_SIZE {
            return Err(Error::InvalidKey);
        }
        let x: [u8; X25519_SIZE] = b[..X25519_SIZE].try_into().unwrap();
        Ok(PublicKey {
            x25519: x.into(),
            ctidh: crate::highctidh::PublicKey::from_bytes(&b[X25519_SIZE..]).map_err(|_| Error::InvalidKey)?,
        })
    }

    /// Encodes the public key.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(PUBLIC_KEY_SIZE);
        out.extend_from_slice(self.x25519.as_bytes());
        out.extend_from_slice(&self.ctidh.to_bytes());
        out
    }
}

impl std::fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PublicKey({:02x?}…)", &self.x25519.as_bytes()[..4])
    }
}

/// A hybrid private key.
#[derive(Clone)]
pub struct PrivateKey {
    x25519: x25519_dalek::StaticSecret,
    ctidh: crate::highctidh::PrivateKey,
}

impl PrivateKey {
    /// Generates a private key.
    pub fn generate() -> Self {
        PrivateKey {
            x25519: x25519_dalek::StaticSecret::random_from_rng(OsRng),
            ctidh: crate::highctidh::PrivateKey::generate(),
        }
    }

    /// Decodes a private key.
    pub fn from_bytes(b: &[u8]) -> Result<Self, Error> {
        if b.len() != PRIVATE_KEY_SIZE {
            return Err(Error::InvalidKey);
        }
        let x: Zeroizing<[u8; X25519_SIZE]> = Zeroizing::new(b[..X25519_SIZE].try_into().unwrap());
        Ok(PrivateKey {
            x25519: x25519_dalek::StaticSecret::from(*x),
            ctidh: crate::highctidh::PrivateKey::from_bytes(&b[X25519_SIZE..]).map_err(|_| Error::InvalidKey)?,
        })
    }

    /// Encodes the private key.
    pub fn to_bytes(&self) -> Zeroizing<Vec<u8>> {
        let mut out = Zeroizing::new(Vec::with_capacity(PRIVATE_KEY_SIZE));
        out.extend_from_slice(self.x25519.as_bytes());
        out.extend_from_slice(&self.ctidh.to_bytes());
        out
    }

    /// Returns the public key.
    pub fn public_key(&self) -> PublicKey {
        PublicKey {
            x25519: (&self.x25519).into(),
            ctidh: self.ctidh.public_key(),
        }
    }

    /// Computes the shared secret with `peer`: the X25519 secret followed by
    /// the CTIDH secret. A degenerate secret, where either part is all
    /// zeros, is an error.
    pub fn derive_secret(&self, peer: &PublicKey) -> Result<Zeroizing<Vec<u8>>, Error> {
        let x = self.x25519.diffie_hellman(&peer.x25519);
        let c = Zeroizing::new(self.ctidh.derive_secret(&peer.ctidh).map_err(|_| Error::DegenerateSecret)?);
        if is_zero(x.as_bytes()) || is_zero(&c[..]) {
            return Err(Error::DegenerateSecret);
        }
        let mut out = Zeroizing::new(Vec::with_capacity(X25519_SIZE + c.len()));
        out.extend_from_slice(x.as_bytes());
        out.extend_from_slice(&c[..]);
        Ok(out)
    }
}

fn is_zero(b: &[u8]) -> bool {
    let zeros = vec![0u8; b.len()];
    b.ct_eq(&zeros).into()
}

/// A keypair.
#[derive(Clone)]
pub struct Keypair {
    pub public: PublicKey,
    pub private: PrivateKey,
}

impl Keypair {
    /// Generates a keypair.
    pub fn generate() -> Self {
        let private = PrivateKey::generate();
        Keypair { public: private.public_key(), private }
    }

    /// Loads the keypair stored as `name.private.pem` and `name.public.pem`
    /// in `dir`, in hpqc's PEM format, or generates and stores one if
    /// neither file exists.
    pub fn load_or_generate(dir: &Path, name: &str) -> Result<Self, Error> {
        let priv_file = dir.join(format!("{name}.private.pem"));
        let pub_file = dir.join(format!("{name}.public.pem"));
        match (priv_file.exists(), pub_file.exists()) {
            (true, true) => {
                let private = PrivateKey::from_bytes(&read_pem(&priv_file, "PRIVATE KEY")?)?;
                let public = PublicKey::from_bytes(&read_pem(&pub_file, "PUBLIC KEY")?)?;
                if private.public_key() != public {
                    return Err(Error::Other("stored public key does not match the private key".into()));
                }
                Ok(Keypair { public, private })
            }
            (false, false) => {
                let kp = Keypair::generate();
                write_pem(&priv_file, "PRIVATE KEY", &kp.private.to_bytes(), 0o600)?;
                write_pem(&pub_file, "PUBLIC KEY", &kp.public.to_bytes(), 0o644)?;
                Ok(kp)
            }
            _ => Err(Error::Other(format!("{name} key files must both exist or both be absent"))),
        }
    }
}

fn pem_label(kind: &str) -> String {
    format!("{} {kind}", SCHEME.to_uppercase())
}

fn read_pem(path: &Path, kind: &str) -> Result<Zeroizing<Vec<u8>>, Error> {
    let text = Zeroizing::new(std::fs::read_to_string(path)?);
    let label = pem_label(kind);
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let body = text
        .split_once(&begin)
        .and_then(|(_, rest)| rest.split_once(&end))
        .map(|(body, _)| body)
        .ok_or_else(|| Error::Other(format!("{}: not a {label}", path.display())))?;
    let b64: Zeroizing<String> = Zeroizing::new(body.chars().filter(|c| !c.is_whitespace()).collect());
    Ok(Zeroizing::new(STANDARD.decode(b64.as_bytes()).map_err(|_| Error::InvalidKey)?))
}

fn write_pem(path: &Path, kind: &str, der: &[u8], mode: u32) -> Result<(), Error> {
    let label = pem_label(kind);
    let b64 = Zeroizing::new(STANDARD.encode(der));
    let mut out = Zeroizing::new(format!("-----BEGIN {label}-----\n"));
    for line in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).unwrap());
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    write_file(path, out.as_bytes(), mode)
}

#[cfg(unix)]
fn write_file(path: &Path, data: &[u8], mode: u32) -> Result<(), Error> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(mode).open(path)?;
    f.write_all(data)?;
    Ok(f.sync_all()?)
}

#[cfg(not(unix))]
fn write_file(path: &Path, data: &[u8], _mode: u32) -> Result<(), Error> {
    Ok(std::fs::write(path, data)?)
}
