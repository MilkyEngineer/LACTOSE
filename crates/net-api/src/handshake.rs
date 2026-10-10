//! The handshake (architecture section 9.1): three hashes a client and a server compare
//! before anything else crosses, and the rule that refuses the connection.
//!
//! Host-only: computed and checked by `app` and `lactose-net` at connection time.

use bytemuck::{Pod, Zeroable};
use lactose_core::{Error, ErrorKind};
use lactose_reflect::{Registry, TypeInfo};

/// The version of the wire format itself: the packet framing, the quantisation constants
/// and the message set. Part of the protocol hash, bumped by `lactose-net` when the
/// framing changes.
pub const PROTOCOL_VERSION: u32 = 1;

/// The hash over the types that cross the wire or are compared on a desync, and the
/// protocol constants (architecture section 9.1). The README states the covered set and
/// the encoding bit for bit; two registries hash equal exactly when every covered type
/// has the same key, version, policy, tracking and layout.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Pod, Zeroable)]
#[repr(transparent)]
pub struct ProtocolHash(pub u64);

impl ProtocolHash {
    /// The hash of `registry`: xxh64, seed zero, over the protocol constants and the
    /// canonical encoding of every type [`covers`](Self::covers) accepts, in ascending
    /// component key order.
    #[must_use]
    pub fn of(registry: &Registry) -> Self {
        todo!()
    }

    /// Whether `info` is covered: its tracking is `replicated`, or its persistence is
    /// `Persistent` or `Transient` (a hashed type, which a desync digest compares). A
    /// `derived` type that is not replicated is not, whatever crate declares it:
    /// presentation types differ between client and server by design.
    #[must_use]
    pub fn covers(info: &TypeInfo) -> bool {
        todo!()
    }
}

/// The boundary build ID (architecture section 9.1): a hash over the `Cargo.lock` entries,
/// the resolved feature set and the source of every boundary crate, computed by the build
/// tooling and embedded in the binary. Named here; `app` supplies the value.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Pod, Zeroable)]
#[repr(transparent)]
pub struct BoundaryBuildId(pub u64);

/// The hash of simulation-relevant cooked data (architecture section 9.1): over the pack
/// index entries of every asset kind marked `SimulationAsset`, computed by the asset
/// store. Named here; `app` supplies the value.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Pod, Zeroable)]
#[repr(transparent)]
pub struct CookedDataHash(pub u64);

/// What each side sends first. Twenty-four bytes of plain data; [`check`](Self::check) is
/// the equality rule.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Pod, Zeroable)]
#[repr(C)]
pub struct Handshake {
    /// The protocol hash of the registry the world was built with.
    pub protocol: ProtocolHash,
    /// The boundary build ID of the binary.
    pub build: BoundaryBuildId,
    /// The cooked-data hash of the store the binary runs against.
    pub cooked: CookedDataHash,
}

impl Handshake {
    /// The length of [`to_bytes`](Self::to_bytes).
    pub const ENCODED_LEN: usize = 24;

    /// The wire form: the three hashes as little-endian `u64`s, in field order.
    #[must_use]
    pub fn to_bytes(self) -> [u8; Self::ENCODED_LEN] {
        let mut bytes = [0; Self::ENCODED_LEN];
        bytes[..8].copy_from_slice(&self.protocol.0.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.build.0.to_le_bytes());
        bytes[16..].copy_from_slice(&self.cooked.0.to_le_bytes());
        bytes
    }

    /// Decodes [`to_bytes`](Self::to_bytes). Every byte string of the right length is a
    /// handshake.
    #[must_use]
    pub fn from_bytes(bytes: &[u8; Self::ENCODED_LEN]) -> Self {
        let word = |at: usize| {
            u64::from_le_bytes(bytes[at..at + 8].try_into().expect("eight bytes of twenty-four"))
        };
        Self {
            protocol: ProtocolHash(word(0)),
            build: BoundaryBuildId(word(8)),
            cooked: CookedDataHash(word(16)),
        }
    }

    /// The equality rule: every hash must match, and the first that does not, in field
    /// order, is the mismatch reported. Either side refuses the connection on an error.
    pub fn check(self, peer: Self) -> core::result::Result<(), HandshakeMismatch> {
        if self.protocol != peer.protocol {
            return Err(HandshakeMismatch::Protocol { ours: self.protocol, theirs: peer.protocol });
        }
        if self.build != peer.build {
            return Err(HandshakeMismatch::Build { ours: self.build, theirs: peer.build });
        }
        if self.cooked != peer.cooked {
            return Err(HandshakeMismatch::Cooked { ours: self.cooked, theirs: peer.cooked });
        }
        Ok(())
    }
}

/// Why a handshake was refused: the first hash that differed.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HandshakeMismatch {
    /// The protocol hashes differ: the wire types or the protocol constants changed.
    Protocol {
        /// Ours.
        ours: ProtocolHash,
        /// The peer's.
        theirs: ProtocolHash,
    },
    /// The boundary build IDs differ: the simulations are not the same build.
    Build {
        /// Ours.
        ours: BoundaryBuildId,
        /// The peer's.
        theirs: BoundaryBuildId,
    },
    /// The cooked-data hashes differ: the simulations run against different content.
    Cooked {
        /// Ours.
        ours: CookedDataHash,
        /// The peer's.
        theirs: CookedDataHash,
    },
}

impl core::fmt::Display for HandshakeMismatch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Protocol { ours, theirs } => {
                write!(f, "protocol hash {:#018x} is not the peer's {:#018x}", ours.0, theirs.0)
            }
            Self::Build { ours, theirs } => {
                write!(f, "boundary build {:#018x} is not the peer's {:#018x}", ours.0, theirs.0)
            }
            Self::Cooked { ours, theirs } => {
                write!(f, "cooked data {:#018x} is not the peer's {:#018x}", ours.0, theirs.0)
            }
        }
    }
}

impl std::error::Error for HandshakeMismatch {}

impl From<HandshakeMismatch> for Error {
    fn from(mismatch: HandshakeMismatch) -> Self {
        Error::new(ErrorKind::VersionMismatch, mismatch.to_string())
    }
}
