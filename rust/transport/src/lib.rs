//! What a group protocol's client asks of Pigeonhole: boxes on streams, and the contact vouchers
//! a new member joins through. Caps and indexes are BACAP's binary forms; a client steps from one
//! index to the next itself, by BACAP.

use std::future::Future;

pub mod sim;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A write to a box that holds something else.
    BoxAlreadyExists,
    Closed,
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::BoxAlreadyExists => {
                write!(f, "transport: box already exists with other contents")
            }
            Error::Closed => write!(f, "transport: closed"),
            Error::Other(e) => write!(f, "transport: {e}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    Data(Vec<u8>),
    NotFound,
    Tombstone,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keypair {
    pub write_cap: Vec<u8>,
    pub read_cap: Vec<u8>,
    /// The index the stream starts at.
    pub first: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Minted {
    pub voucher: Vec<u8>,
    pub payload: Vec<u8>,
    pub write_cap: Vec<u8>,
    pub read_cap: Vec<u8>,
    pub secret_key: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inducted {
    pub name: String,
    pub mutated_read_cap: Vec<u8>,
    pub sealed_reply: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    pub who_reply: Vec<u8>,
    pub mutated_write_cap: Vec<u8>,
}

/// Pigeonhole's storage, and contact vouchers: a joiner mints one for its stream, an existing
/// member inducts it with a sealed reply, and the joiner opens the reply. Every call may fail; a
/// caller that must succeed retries.
pub trait Backend: Send + Sync + 'static {
    fn new_keypair(&self) -> impl Future<Output = Result<Keypair>> + Send;
    /// Write `plaintext` to the box at `index`; an empty plaintext is a tombstone. Writing again
    /// what a box holds succeeds; writing anything else is [`Error::BoxAlreadyExists`].
    fn write_box(
        &self,
        write_cap: &[u8],
        index: &[u8],
        plaintext: &[u8],
    ) -> impl Future<Output = Result<()>> + Send;
    /// Read the box at `index`, reporting at once a box not written.
    fn read_box(&self, read_cap: &[u8], index: &[u8]) -> impl Future<Output = Result<Read>> + Send;
    /// The storage replicas' current epoch.
    fn replica_epoch(&self) -> impl Future<Output = Result<u64>> + Send;
    /// The most a box holds.
    fn max_plaintext(&self) -> impl Future<Output = Result<usize>> + Send;
    fn voucher_mint(
        &self,
        write_cap: &[u8],
        name: &str,
    ) -> impl Future<Output = Result<Minted>> + Send;
    /// The voucher stream's write cap and read cap.
    fn voucher_derive_stream(
        &self,
        voucher: &[u8],
    ) -> impl Future<Output = Result<(Vec<u8>, Vec<u8>)>> + Send;
    fn voucher_induct(
        &self,
        voucher: &[u8],
        payload: &[u8],
        who_reply: &[u8],
    ) -> impl Future<Output = Result<Inducted>> + Send;
    fn voucher_open(
        &self,
        secret_key: &[u8],
        sealed_reply: &[u8],
        write_cap: &[u8],
    ) -> impl Future<Output = Result<Opened>> + Send;
}
