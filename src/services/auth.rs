//! Request MACs. A service's requests to a router or a directory carry a
//! MAC under a key both sides derive from a NIKE exchange between the
//! service's key and the plugin's published key:
//!
//! ```text
//! ss  = DH(servicePriv, pluginPub)
//! key = HKDF-BLAKE2b-256(ss, info = label || pluginPub || servicePub)
//! mac = BLAKE2b-256(key = key, msg = canonical CBOR of the request with MAC null)
//! ```

use blake2::Blake2bMac;
use blake2::digest::Mac;
use blake2::digest::consts::U32;
use zeroize::Zeroizing;

use crate::services::Error;
use crate::services::cbor::{self, Value};
use crate::services::nike::{self, Keypair, PrivateKey, PublicKey};
use crate::services::sealedbox::Hkdf;
use crate::services::wire::{AdvertiseRequest, RegisterRequest, ServiceId, WithdrawRequest};

const LABEL: &[u8] = b"katzenpost-pigeonservice-mac-v1";

fn mac_key(key: &Keypair, plugin: &PublicKey) -> Result<Zeroizing<[u8; 32]>, Error> {
    derive_mac_key(&key.private, plugin, &plugin.to_bytes(), &key.public.to_bytes())
}

fn derive_mac_key(private: &PrivateKey, peer: &PublicKey, plugin_pub: &[u8], service_pub: &[u8]) -> Result<Zeroizing<[u8; 32]>, Error> {
    let ss = private.derive_secret(peer)?;
    let mut info = LABEL.to_vec();
    info.extend_from_slice(plugin_pub);
    info.extend_from_slice(service_pub);
    let mut out = Zeroizing::new([0u8; 32]);
    Hkdf::new(None, &ss).expand(&info, &mut out[..]).map_err(|_| Error::Crypto)?;
    Ok(out)
}

fn mac_of(key: &[u8; 32], unsigned: &Value) -> Vec<u8> {
    let mut m = <Blake2bMac<U32> as Mac>::new_from_slice(key).expect("32-byte BLAKE2b key");
    m.update(&cbor::encode(unsigned));
    m.finalize().into_bytes().to_vec()
}

/// Fills in a RegisterRequest's MAC for the router whose key is `router`.
pub fn sign_register(key: &Keypair, router: &PublicKey, r: &mut RegisterRequest) -> Result<(), Error> {
    r.mac = None;
    r.mac = Some(mac_of(&*mac_key(key, router)?, &r.to_value()));
    Ok(())
}

/// Fills in an AdvertiseRequest's MAC for the directory whose key is `dir`.
pub fn sign_advertise(key: &Keypair, dir: &PublicKey, r: &mut AdvertiseRequest) -> Result<(), Error> {
    r.mac = None;
    r.mac = Some(mac_of(&*mac_key(key, dir)?, &r.to_value()));
    Ok(())
}

/// Fills in a WithdrawRequest's MAC for the directory whose key is `dir`.
pub fn sign_withdraw(key: &Keypair, dir: &PublicKey, r: &mut WithdrawRequest) -> Result<(), Error> {
    r.mac = None;
    r.mac = Some(mac_of(&*mac_key(key, dir)?, &r.to_value()));
    Ok(())
}

/// Checks a request MAC as the plugin, for the service whose public key is
/// `service_pub`.
fn verify(plugin: &Keypair, service_pub: &[u8], unsigned: &Value, mac: Option<&[u8]>) -> Result<(), Error> {
    use subtle::ConstantTimeEq;
    let peer = PublicKey::from_bytes(service_pub).map_err(|_| Error::BadMac)?;
    let key = derive_mac_key(&plugin.private, &peer, &plugin.public.to_bytes(), service_pub).map_err(|_| Error::BadMac)?;
    let want = mac_of(&key, unsigned);
    match mac {
        Some(m) if bool::from(want.ct_eq(m)) => Ok(()),
        _ => Err(Error::BadMac),
    }
}

/// Checks a RegisterRequest's MAC as the router whose key is `router`.
pub fn verify_register(router: &Keypair, r: &RegisterRequest) -> Result<(), Error> {
    if r.nike_scheme != nike::SCHEME {
        return Err(Error::BadMac);
    }
    let unsigned = RegisterRequest { mac: None, ..r.clone() };
    verify(router, &r.public_key, &unsigned.to_value(), r.mac.as_deref())
}

/// Checks an AdvertiseRequest's MAC as the directory whose key is `dir`.
pub fn verify_advertise(dir: &Keypair, r: &AdvertiseRequest) -> Result<(), Error> {
    if r.descriptor.nike_scheme != nike::SCHEME {
        return Err(Error::BadMac);
    }
    let unsigned = AdvertiseRequest { mac: None, ..r.clone() };
    verify(dir, &r.descriptor.public_key, &unsigned.to_value(), r.mac.as_deref())
}

/// Checks a WithdrawRequest's MAC as the directory whose key is `dir`, for
/// the service whose public key is `service_pub`.
pub fn verify_withdraw(dir: &Keypair, service_pub: &[u8], r: &WithdrawRequest) -> Result<(), Error> {
    if ServiceId::from_public_key(service_pub) != r.service_id {
        return Err(Error::BadMac);
    }
    let unsigned = WithdrawRequest { mac: None, ..r.clone() };
    verify(dir, service_pub, &unsigned.to_value(), r.mac.as_deref())
}
