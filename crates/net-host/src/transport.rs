//! The transport (architecture sections 5.1 and 9.2): unreliable, unordered datagrams
//! between connections, small enough that a loopback or a UDP implementation is a page.
//! `lactose-net` builds reliability, ordering and the packet formats on top; the
//! milestone 1 netcode spike runs over a throwaway implementation of this trait.
//!
//! Host-only: owned and driven by the dedicated network thread.

use std::time::Duration;

use lactose_core::{Error, ErrorKind, Result};

/// The largest payload one datagram carries. Under the usual path MTU with headroom for
/// the transport's own headers, so a datagram is never fragmented.
pub const MAX_DATAGRAM_BYTES: usize = 1200;

/// A connection, as the transport names it: unique for the life of the transport, never
/// reused after `Disconnected`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ConnectionId(pub u32);

/// One datagram: whole or not at all, possibly reordered or duplicated, never corrupted
/// (the transport's own checksum drops a damaged one).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Datagram {
    /// The connection it came from.
    pub connection: ConnectionId,
    payload: Vec<u8>,
}

impl Datagram {
    /// A datagram carrying `payload` from `connection`. Fails with `InvalidData` if
    /// `payload` is longer than [`MAX_DATAGRAM_BYTES`].
    pub fn new(connection: ConnectionId, payload: Vec<u8>) -> Result<Self> {
        if payload.len() > MAX_DATAGRAM_BYTES {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("a datagram carries at most {MAX_DATAGRAM_BYTES} bytes"),
            ));
        }
        Ok(Self { connection, payload })
    }

    /// The payload.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// The payload, owned.
    #[must_use]
    pub fn into_payload(self) -> Vec<u8> {
        self.payload
    }
}

/// Why a connection ended.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DisconnectReason {
    /// This side called [`Transport::disconnect`].
    Local,
    /// The peer closed it.
    Remote,
    /// Nothing arrived from the peer for the transport's timeout.
    TimedOut,
    /// The socket or the link failed.
    Failed,
}

/// What [`Transport::poll`] yields.
#[derive(Clone, Debug)]
pub enum TransportEvent {
    /// A connection is established: the first event for its ID.
    Connected(ConnectionId),
    /// A connection ended: the last event for its ID.
    Disconnected {
        /// The connection.
        connection: ConnectionId,
        /// Why.
        reason: DisconnectReason,
    },
    /// A datagram arrived on an established connection.
    Received(Datagram),
}

/// What a transport measures about one connection, locally.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LinkStats {
    /// The smoothed round-trip estimate, or `None` for a transport that does not measure
    /// one (`net` then estimates from its own acknowledgements). A loopback reports zero.
    pub rtt: Option<Duration>,
    /// The recent loss rate in thousandths, or `None` if not measured.
    pub loss_permille: Option<u16>,
    /// Datagrams sent on the connection.
    pub datagrams_sent: u64,
    /// Datagrams received on the connection.
    pub datagrams_received: u64,
    /// Payload bytes sent.
    pub bytes_sent: u64,
    /// Payload bytes received.
    pub bytes_received: u64,
}

/// Unreliable, unordered datagrams with a connection identity. How a transport
/// establishes connections (listening, dialling an address) is its constructor's; the
/// trait sees connections only as events. The network thread owns the transport and is
/// the only caller; nothing here blocks.
pub trait Transport: Send + 'static {
    /// Sends `payload` to `to` as one datagram, or drops it: the call never blocks and
    /// never waits for delivery. Fails with `InvalidData` if `payload` is longer than
    /// [`MAX_DATAGRAM_BYTES`] (nothing is sent), `NotFound` if `to` is not an established
    /// connection, and `Network` if the socket failed.
    fn send(&mut self, to: ConnectionId, payload: &[u8]) -> Result<()>;

    /// Appends every event that is ready, in the order the transport observed them, and
    /// returns at once. For one connection, `Connected` comes first, `Received` only
    /// between it and `Disconnected`, and `Disconnected` last.
    fn poll(&mut self, events: &mut Vec<TransportEvent>);

    /// The local statistics of `connection`, or `None` if it is not established.
    fn stats(&self, connection: ConnectionId) -> Option<LinkStats>;

    /// Closes `connection`. A later `poll` yields `Disconnected` with `Local` for it;
    /// unknown connections are ignored.
    fn disconnect(&mut self, connection: ConnectionId);
}
