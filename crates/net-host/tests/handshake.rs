//! The handshake (README, "The handshake"): the protocol hash is the stated byte stream,
//! pinned for a small registry and for the engine's own registry; each change the README
//! names changes it and an unhashed, unreplicated type does not; `check` reports the
//! first difference in field order; the wire form round-trips.

#![forbid(unsafe_code)]

use lactose_core::{ErrorKind, Result, Tick, xxh64};
use lactose_net_host::handshake::{
    BoundaryBuildId, CookedDataHash, Handshake, HandshakeMismatch, PROTOCOL_VERSION, ProtocolHash,
};
use lactose_net_host::quant::{POSITION_UNITS_PER_METRE, ROTATION_SCALE};
use lactose_net_host::transport::MAX_DATAGRAM_BYTES;
use lactose_reflect::{Padding, Reflect, Registry, component};
use lactose_sim::{TICK_RATE_HZ, TypeTable};

/// The base type every variant below changes in one way.
mod base {
    use super::*;

    #[component(key = "handshake::body", persistent, replicated)]
    pub struct Body {
        #[field(id = 1)]
        pub mass: u32,
        #[field(id = 2)]
        pub charge: i32,
    }
}

mod id_changed {
    use super::*;

    #[component(key = "handshake::body", persistent, replicated)]
    pub struct Body {
        #[field(id = 1)]
        pub mass: u32,
        #[field(id = 3)]
        pub charge: i32,
    }
}

mod offset_changed {
    use super::*;

    #[component(key = "handshake::body", persistent, replicated)]
    pub struct Body {
        #[field(id = 2)]
        pub charge: i32,
        #[field(id = 1)]
        pub mass: u32,
    }
}

mod size_changed {
    use super::*;

    #[component(key = "handshake::body", persistent, replicated)]
    pub struct Body {
        #[field(id = 1)]
        pub mass: u32,
        #[field(id = 2)]
        pub charge: i16,
        #[field(padding)]
        pub _pad: Padding<2>,
    }
}

/// `Tick` is a `u32` under another type tag, and of the same kind.
mod tag_changed {
    use super::*;

    #[component(key = "handshake::body", persistent, replicated)]
    pub struct Body {
        #[field(id = 1)]
        pub mass: Tick,
        #[field(id = 2)]
        pub charge: i32,
    }
}

mod kind_changed {
    use super::*;

    #[component(key = "handshake::body", persistent, replicated)]
    pub struct Body {
        #[field(id = 1)]
        pub mass: u32,
        #[field(id = 2)]
        pub charge: u32,
    }
}

mod version_raised {
    use super::*;

    #[component(key = "handshake::body", persistent, replicated, version = 2)]
    pub struct Body {
        #[field(id = 1)]
        pub mass: u32,
        #[field(id = 2)]
        pub charge: i32,
    }
}

/// A persistent type in the small registry.
#[component(key = "handshake::label", persistent)]
pub struct Label(#[field(id = 1)] pub u32);

/// Derived but replicated: covered.
#[component(key = "handshake::replicated_view", derived, replicated)]
pub struct ReplicatedView(#[field(id = 1)] pub u32);

/// Persistent: covered.
#[component(key = "handshake::persistent", persistent)]
pub struct Persistent(#[field(id = 1)] pub u32);

/// Transient: covered.
#[component(key = "handshake::transient", transient)]
pub struct Transient(#[field(id = 1)] pub u32);

/// Derived and extracted only: not covered.
#[component(key = "handshake::presentation", derived, extracted)]
pub struct Presentation(#[field(id = 1)] pub u32);

/// Derived only: not covered.
#[component(key = "handshake::local", derived)]
pub struct Local(#[field(id = 1)] pub u32);

/// The small registry: the base body and a persistent label.
fn small<B: Reflect>() -> Registry {
    let mut registry = Registry::new();
    registry.register::<B>().expect("registers");
    registry.register::<Label>().expect("registers");
    registry
}

fn with<T: Reflect>(mut registry: Registry) -> Registry {
    registry.register::<T>().expect("registers");
    registry
}

/// The stream the README states, built here from its sentences.
fn stated_stream(registry: &Registry) -> Vec<u8> {
    let mut stream = Vec::new();
    stream.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    stream.extend_from_slice(&TICK_RATE_HZ.to_le_bytes());
    stream.extend_from_slice(&POSITION_UNITS_PER_METRE.to_le_bytes());
    stream.extend_from_slice(&i32::from(ROTATION_SCALE).to_le_bytes());
    stream.extend_from_slice(&u32::try_from(MAX_DATAGRAM_BYTES).expect("small").to_le_bytes());
    let mut covered: Vec<_> = registry.iter().filter(|info| ProtocolHash::covers(info)).collect();
    covered.sort_by_key(|info| info.key());
    for info in covered {
        info.write_schema(&mut stream);
    }
    stream
}

#[test]
fn the_hash_is_xxh64_of_the_stated_stream() {
    let engine = engine();
    for registry in [&Registry::new(), &small::<base::Body>(), engine.registry()] {
        assert_eq!(ProtocolHash::of(registry), ProtocolHash(xxh64(&stated_stream(registry), 0)));
    }
    // The constants block alone, as the README lists it: 1, 60, 1024, 32767, 1200.
    let constants: Vec<u8> =
        [1_u32, 60, 1024, 32_767, 1200].iter().flat_map(|value| value.to_le_bytes()).collect();
    assert_eq!(ProtocolHash::of(&Registry::new()), ProtocolHash(xxh64(&constants, 0)));
    // Any constant changed changes the hash.
    let mut changed = constants;
    changed[4] = 30;
    assert_ne!(ProtocolHash::of(&Registry::new()), ProtocolHash(xxh64(&changed, 0)));
}

#[test]
fn the_hash_of_a_small_registry_is_pinned() {
    assert_eq!(ProtocolHash::of(&Registry::new()), ProtocolHash(0x8184_0AD9_C583_EB01));
    assert_eq!(ProtocolHash::of(&small::<base::Body>()), ProtocolHash(0x3CCB_5B16_AE7D_00C4));
}

/// Every engine type a game's registry holds before its own: the sim's and the world's,
/// `lactose-net-api`'s, `lactose-input`'s and `lactose-physics-api`'s. A change to any of
/// them is a protocol break and moves this pin (verification row 91).
fn engine() -> TypeTable {
    let mut types = TypeTable::new();
    let registered: [Result<()>; 3] = [
        lactose_net_api::register_types(&mut types),
        lactose_input::register_types(&mut types),
        lactose_physics_api::register_types(&mut types),
    ];
    for result in registered {
        result.expect("the engine's types register");
    }
    types
}

#[test]
fn the_hash_of_the_engine_registry_is_pinned() {
    let types = engine();
    // In ascending key order, which is the order the stream covers them in.
    let covered: Vec<&str> = types
        .registry()
        .iter()
        .filter(|info| ProtocolHash::covers(info))
        .map(|info| info.name())
        .collect();
    let uncovered: Vec<&str> = types
        .registry()
        .iter()
        .filter(|info| !ProtocolHash::covers(info))
        .map(|info| info.name())
        .collect();
    assert_eq!(
        covered,
        [
            "physics::joint",
            "physics::teleported",
            "ecs::predicted_origin",
            "physics::rigid_body",
            "input::player_input",
            "physics::velocity",
            "ecs::owned_by",
            "physics::transform",
            "physics::collider",
            "sim::time_scale",
        ]
    );
    assert_eq!(
        uncovered,
        [
            "sim::streaming_anchor",
            "net::remote_slot",
            "sim::tick_delta",
            "physics::joint_handle",
            "physics::body_handle",
            "ecs::in_region",
            "ecs::claimed_by",
            "sim::current_tick",
            "sim::region_policy",
        ]
    );
    assert_eq!(ProtocolHash::of(types.registry()), ProtocolHash(0xE128_FEA4_5092_6B24));
}

#[test]
fn each_named_change_changes_the_hash() {
    let base = ProtocolHash::of(&small::<base::Body>());
    let changed = [
        ("a field's ID", ProtocolHash::of(&small::<id_changed::Body>())),
        ("a field's offset", ProtocolHash::of(&small::<offset_changed::Body>())),
        ("a field's size", ProtocolHash::of(&small::<size_changed::Body>())),
        ("a field's type tag", ProtocolHash::of(&small::<tag_changed::Body>())),
        ("a field's kind", ProtocolHash::of(&small::<kind_changed::Body>())),
        ("the version", ProtocolHash::of(&small::<version_raised::Body>())),
        (
            "a replicated type added",
            ProtocolHash::of(&with::<ReplicatedView>(small::<base::Body>())),
        ),
        ("a persistent type added", ProtocolHash::of(&with::<Persistent>(small::<base::Body>()))),
        ("a transient type added", ProtocolHash::of(&with::<Transient>(small::<base::Body>()))),
    ];
    for (what, hash) in changed {
        assert_ne!(hash, base, "{what}");
    }
    let unchanged = [
        (
            "a derived, extracted type added",
            ProtocolHash::of(&with::<Presentation>(small::<base::Body>())),
        ),
        ("a derived type added", ProtocolHash::of(&with::<Local>(small::<base::Body>()))),
    ];
    for (what, hash) in unchanged {
        assert_eq!(hash, base, "{what}");
    }
}

#[test]
fn the_covered_set_is_replicated_or_hashed() {
    let registry =
        with::<Local>(with::<Presentation>(with::<Transient>(with::<ReplicatedView>(small::<
            base::Body,
        >()))));
    let covered: Vec<bool> = [
        base::Body::KEY,
        Label::KEY,
        ReplicatedView::KEY,
        Transient::KEY,
        Presentation::KEY,
        Local::KEY,
    ]
    .iter()
    .map(|&key| ProtocolHash::covers(registry.get(key).expect("registered")))
    .collect();
    assert_eq!(covered, [true, true, true, true, false, false]);
}

#[test]
fn check_reports_the_first_difference_in_field_order() {
    let ours = Handshake {
        protocol: ProtocolHash(1),
        build: BoundaryBuildId(2),
        cooked: CookedDataHash(3),
    };
    assert_eq!(ours.check(ours), Ok(()));
    let all_differ = Handshake {
        protocol: ProtocolHash(10),
        build: BoundaryBuildId(20),
        cooked: CookedDataHash(30),
    };
    assert_eq!(
        ours.check(all_differ),
        Err(HandshakeMismatch::Protocol { ours: ProtocolHash(1), theirs: ProtocolHash(10) })
    );
    let build_and_cooked = Handshake { protocol: ours.protocol, ..all_differ };
    assert_eq!(
        ours.check(build_and_cooked),
        Err(HandshakeMismatch::Build { ours: BoundaryBuildId(2), theirs: BoundaryBuildId(20) })
    );
    let cooked = Handshake { cooked: CookedDataHash(30), ..ours };
    let mismatch = ours.check(cooked).expect_err("the cooked hash differs");
    assert_eq!(
        mismatch,
        HandshakeMismatch::Cooked { ours: CookedDataHash(3), theirs: CookedDataHash(30) }
    );
    assert_eq!(lactose_core::Error::from(mismatch).kind(), ErrorKind::VersionMismatch);
}

#[test]
fn the_wire_form_round_trips() {
    let handshake = Handshake {
        protocol: ProtocolHash(0x0123_4567_89AB_CDEF),
        build: BoundaryBuildId(u64::MAX),
        cooked: CookedDataHash(7),
    };
    let bytes = handshake.to_bytes();
    assert_eq!(bytes[..8], 0x0123_4567_89AB_CDEF_u64.to_le_bytes());
    assert_eq!(Handshake::from_bytes(&bytes), handshake);
    let arbitrary: [u8; Handshake::ENCODED_LEN] = core::array::from_fn(|i| (i * 37) as u8);
    assert_eq!(Handshake::from_bytes(&arbitrary).to_bytes(), arbitrary);
}
