//! The host side of networking (architecture section 4, `net-api`; sections 5.3, 9.1,
//! 9.2 and 9.3): the quantised wire values, the handshake, the transport trait a
//! loopback or UDP implementation is a page of, the packet framing and the per-connection
//! handles, the per-client interest state the packer keeps, the interpolation buffer
//! the presentation schedule samples through a sequence lock per slot, and the named
//! places for clock sync and the control messages. The simulation side, what enters and
//! leaves a world, is `lactose-net-api`, inside the boundary.
//!
//! Foundation tier, outside the determinism boundary, and no boundary crate may depend on
//! it (`crate-tiers.toml`): floats, clocks and sockets live here, and the only things that
//! cross into the simulation are the records of `lactose-net-api`, which the network
//! thread builds from what arrives here. The `README.md` is the contract.
//!
//! # Status
//!
//! Implemented for milestone 1. The packer and the receiver that use these types are
//! `lactose-net`'s, milestone 4; the README states the rules they keep.
//!
//! # Where to start
//!
//! - [`Handshake`] is what each side sends first; [`transport::Transport`] the datagram
//!   trait; [`packet`] the framing shapes and [`packet::ConnectionHandles`] the table both
//!   ends keep; [`interest::ClientInterest`] the per-client state the packer keeps against
//!   the gather's frame.
//! - [`QuantisedPosition`] and [`QuantisedRotation`] are the wire values, with the
//!   conversions through `lactose_math`.
//! - [`InterpolationBuffer`] splits into the writer the network thread pushes received
//!   states into and the reader the presentation schedule samples by
//!   `lactose_net_api::InterpolationSlot`.
//! - [`clock::ClockEstimate`] and [`message`] are named places.
//!
//! ```
//! use lactose_core::Tick;
//! use lactose_math::{Quat, Scalar, Vec3};
//! use lactose_net_host::InterpolationBuffer;
//! use lactose_net_host::interpolation::{
//!     PresentationTime, ReceivedState, RemoteSample, SampleStatus,
//! };
//!
//! // The network thread keeps the writer; the presentation schedule holds a reader and
//! // samples every frame into its own buffer, without a lock.
//! let (mut writer, reader) = InterpolationBuffer::split(4096);
//! let slot = writer.allocate()?;
//! for (tick, x) in [(10, 0), (12, 2)] {
//!     let position = Vec3::new(Scalar::from_i32(x), Scalar::ZERO, Scalar::ZERO);
//!     let state =
//!         ReceivedState { tick: Tick(tick), teleported: Tick(0), position, rotation: Quat::IDENTITY };
//!     writer.push(slot, state)?;
//! }
//! let mut samples = [RemoteSample::MISSING; 1];
//! reader.sample_batch(&[slot], PresentationTime { tick: Tick(11), fraction: 0 }, &mut samples);
//! assert_eq!(samples[0].status, SampleStatus::Interpolated);
//! assert_eq!(samples[0].position.x, Scalar::ONE);
//! # Ok::<(), lactose_core::Error>(())
//! ```

#![forbid(unsafe_code)]

pub mod clock;
pub mod handshake;
pub mod interest;
pub mod interpolation;
pub mod message;
pub mod packet;
pub mod quant;
pub mod transport;

pub use clock::ClockEstimate;
pub use handshake::Handshake;
pub use interest::ClientInterest;
pub use interpolation::{InterpolationBuffer, InterpolationReader, InterpolationWriter};
pub use packet::EntityHandle;
pub use quant::{QuantisedPosition, QuantisedRotation};
pub use transport::Transport;

/// What the contract promises at the type level, pinned so the implementation cannot lose
/// it without a compile error: the buffer is shared by its writer and readers across
/// threads; a transport is owned by one thread; the per-client state and the handle table
/// travel to the packing thread; the wire-shaped values are plain data of the stated
/// sizes; and the constants agree with the sections that fix them.
const _: () = {
    use bytemuck::Pod;

    const fn send_sync<T: Send + Sync>() {}
    const fn send<T: Send + ?Sized>() {}
    const fn plain<T: Pod + Send + Sync + 'static>() {}

    send_sync::<interest::ClientInterest>();
    send_sync::<packet::ConnectionHandles>();
    send_sync::<packet::PacketRing>();
    send_sync::<interpolation::InterpolationBuffer>();
    send::<interpolation::InterpolationWriter>();
    send_sync::<interpolation::InterpolationReader>();
    send_sync::<interpolation::RemoteSample>();
    send_sync::<clock::ClockEstimate>();
    send_sync::<transport::Datagram>();
    send_sync::<transport::TransportEvent>();
    send_sync::<message::JoinAccepted>();
    send::<dyn transport::Transport>();

    plain::<handshake::Handshake>();
    plain::<handshake::ProtocolHash>();
    plain::<packet::EntityHandle>();
    plain::<packet::PacketSequence>();
    plain::<QuantisedPosition>();
    plain::<QuantisedRotation>();

    assert!(size_of::<handshake::Handshake>() == handshake::Handshake::ENCODED_LEN);
    assert!(size_of::<packet::EntityHandle>() == 4);
    assert!(size_of::<QuantisedPosition>() == 12);
    assert!(size_of::<QuantisedRotation>() == 8);
    assert!(quant::POSITION_UNITS_PER_METRE > 0);
    assert!((quant::POSITION_UNITS_PER_METRE as u32).is_power_of_two());
    assert!(transport::MAX_DATAGRAM_BYTES <= u16::MAX as usize);
    assert!(clock::MAX_RATE_ADJUST_THOUSANDTHS == 30);
    assert!(packet::AckField::WINDOW == 32);
    assert!(packet::EntityHandle::NONE.is_none());
};
