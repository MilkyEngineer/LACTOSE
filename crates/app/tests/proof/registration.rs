//! "The registration order": three sketch crates whose functions record their calls, and
//! the input applier every world gets.

use std::num::NonZeroU64;
use std::sync::{Arc, Mutex, PoisonError};

use lactose_app::{
    Cooked, CrateRegistration, Game, Start, WorldConfig, build_client, build_server,
};
use lactose_core::{Error, ErrorKind, NetId, PlayerId, Result, StableKey, Tick};
use lactose_ecs::{RegionId, World};
use lactose_input::{PlayerInput, Yaw};
use lactose_net_api::GatherState;
use lactose_net_host::handshake::ProtocolHash;
use lactose_net_host::message::JoinAccepted;
use lactose_physics_api::Physics;
use lactose_sim::{PlayerSample, Registration, SessionEvent, TickInput, TypeTable};

use crate::common::{Content, executor, load, server, session, step};

static CALLS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn call(what: String) {
    CALLS.lock().unwrap_or_else(PoisonError::into_inner).push(what);
}

fn take() -> Vec<String> {
    std::mem::take(&mut *CALLS.lock().unwrap_or_else(PoisonError::into_inner))
}

/// What a setup sees of the engine's setups, which run before every crate's.
fn engine(world: &World) -> String {
    let physics = world.service::<Physics>().is_some();
    let gather = world.service::<GatherState>().is_some();
    format!("physics={physics} gather={gather}")
}

fn refused(what: &str) -> Result<()> {
    Err(Error::new(ErrorKind::InvalidData, format!("{what} refused")))
}

fn a_types(_: &mut TypeTable) -> Result<()> {
    call("a.types".into());
    Ok(())
}

fn a_setup(world: &mut World) -> Result<()> {
    call(format!("a.setup {}", engine(world)));
    Ok(())
}

fn a_systems(_: &mut Registration) {
    call("a.systems".into());
}

fn b_types(_: &mut TypeTable) -> Result<()> {
    call("b.types".into());
    Ok(())
}

fn b_systems(_: &mut Registration) {
    call("b.systems".into());
}

fn c_types(_: &mut TypeTable) -> Result<()> {
    call("c.types".into());
    Ok(())
}

fn c_failing_types(_: &mut TypeTable) -> Result<()> {
    call("c.types".into());
    refused("the types")
}

fn c_setup(world: &mut World) -> Result<()> {
    call(format!("c.setup {}", engine(world)));
    Ok(())
}

fn c_failing_setup(_: &mut World) -> Result<()> {
    refused("the setup")
}

fn c_systems(_: &mut Registration) {
    call("c.systems".into());
}

fn order(_: &mut Registration) {
    call("order".into());
}

static A: CrateRegistration = CrateRegistration {
    name: "a",
    types: a_types,
    setup: Some(a_setup),
    systems: a_systems,
    migrations: None,
};
static B: CrateRegistration = CrateRegistration {
    name: "b",
    types: b_types,
    setup: None,
    systems: b_systems,
    migrations: None,
};
static C: CrateRegistration = CrateRegistration {
    name: "c",
    types: c_types,
    setup: Some(c_setup),
    systems: c_systems,
    migrations: None,
};
static C_TYPES_FAIL: CrateRegistration = CrateRegistration {
    name: "c",
    types: c_failing_types,
    setup: None,
    systems: c_systems,
    migrations: None,
};
static C_SETUP_FAILS: CrateRegistration = CrateRegistration {
    name: "c",
    types: b_types,
    setup: Some(c_failing_setup),
    systems: c_systems,
    migrations: None,
};

static THREE: Game = Game { name: "three", crates: &[A, B, C], order: Some(order), sessions: None };

static TYPES_FAIL: Game =
    Game { name: "types", crates: &[A, B, C_TYPES_FAIL], order: None, sessions: None };
static SETUP_FAILS: Game =
    Game { name: "setup", crates: &[A, C_SETUP_FAILS], order: None, sessions: None };

fn content() -> Arc<dyn Cooked> {
    Content::new()
}

#[test]
fn types_setups_and_systems_run_in_the_stated_order_in_every_kind() {
    let executor = executor(2);
    let server = build_server(&executor, &THREE, WorldConfig::headless(), Start::Fresh, content())
        .expect("the server builds");
    let built_server = take();
    let accepted = JoinAccepted { player: PlayerId(1), tick: Tick(0) };
    let entity = StableKey::Net(NetId::runtime(NonZeroU64::MIN).expect("runtime"));
    let client =
        build_client(&executor, &THREE, WorldConfig::headless(), accepted, entity, 2, content())
            .expect("the client builds");
    let built_client = take();
    let expected = [
        "a.types",
        "b.types",
        "c.types",
        "a.setup physics=true gather=true",
        "c.setup physics=true gather=true",
        "a.systems",
        "b.systems",
        "c.systems",
        "order",
    ];
    assert_eq!(built_server, expected);
    assert_eq!(built_client, expected);
    client.shutdown();
    server.shutdown();
    executor.shutdown();
}

#[test]
fn a_failing_registration_names_its_crate() {
    let error = TYPES_FAIL.types().err().expect("c's types fail");
    assert!(error.message().contains("`c`"), "{error}");
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    assert_eq!(take(), ["a.types", "b.types", "c.types"], "the first error ends the fold");

    let executor = executor(2);
    let error =
        build_server(&executor, &SETUP_FAILS, WorldConfig::headless(), Start::Fresh, content())
            .err()
            .expect("c's setup fails");
    assert!(error.message().contains("`c`"), "{error}");
    executor.shutdown();
}

#[test]
fn a_log_retention_below_the_cap_is_refused() {
    let executor = executor(2);
    let config =
        WorldConfig { log_retention: lactose_app::CATCH_UP_CAP - 1, ..WorldConfig::headless() };
    let error =
        build_server(&executor, &THREE, config, Start::Fresh, content()).err().expect("refused");
    assert_eq!(error.kind(), ErrorKind::InvalidData);
    executor.shutdown();
}

/// A server world gets `PlayerInputApplier`: a frame's sample reaches the player's entity,
/// found the one way every applier finds it.
#[test]
fn a_frames_input_reaches_the_players_entity() {
    let executor = executor(2);
    let content = Content::new();
    let server = server(&executor, WorldConfig::headless(), Start::Fresh, &content);
    let (server, _) = step(&executor, server, load(&content, [1]));
    let (server, out) =
        step(&executor, server, session(1, SessionEvent::Join { region: RegionId(1) }));
    assert!(out.failures.is_empty(), "{out:?}");
    let input = PlayerInput { yaw: Yaw(7), ..PlayerInput::default() };
    let sample = PlayerSample {
        input_tick: Tick(server.tick().0 + 1),
        substituted: false,
        sample: input.to_sample_bytes().to_vec(),
    };
    let mut frame = TickInput::default();
    frame.inputs.insert(PlayerId(1), sample);
    let (server, out) = step(&executor, server, frame);
    assert!(out.failures.is_empty(), "{out:?}");
    let world = server.simulation().world();
    let key = lactose_input::player_entity(world, PlayerId(1)).expect("the player's entity");
    assert_eq!(world.get::<PlayerInput>(key), Some(&input));
    server.shutdown();
    executor.shutdown();
}

/// The engine's registry, as `Game::types` builds it for a game with no crates of its own:
/// every engine crate's `register_types`, in the README's order (the verification row
/// "`lactose-net-api` is implemented"). A change to any covered type moves the hash; the
/// name lists catch a change of policy, a type moving between covered and not.
/// `lactose-net-host`'s own pin covers only the engine crates it depends on, which leaves
/// out `lactose-anim`, `lactose-present-api` and `lactose-predict`.
#[test]
fn the_hash_of_the_engine_registry_is_pinned() {
    let empty = Game { name: "empty", crates: &[], order: None, sessions: None };
    let types = empty.types().unwrap();
    let registry = types.registry();
    // In ascending key order, which is the order the stream covers them in.
    let named = |covered: bool| -> Vec<&str> {
        registry
            .iter()
            .filter(|info| ProtocolHash::covers(info) == covered)
            .map(|info| info.name())
            .collect()
    };
    let covered = [
        "anim::sim_pose",
        "present::light",
        "physics::joint",
        "physics::teleported",
        "ecs::predicted_origin",
        "anim::state",
        "physics::rigid_body",
        "anim::bones",
        "input::player_input",
        "physics::velocity",
        "anim::bone_link",
        "ecs::owned_by",
        "present::camera_rig",
        "physics::transform",
        "physics::collider",
        "anim::params",
        "present::mesh",
        "anim::rig",
        "sim::time_scale",
        "anim::root_motion",
        "present::decal",
    ];
    assert_eq!(named(true), covered);
    // `derived` and not replicated: never sent and never compared.
    let uncovered = [
        "sim::streaming_anchor",
        "net::remote_slot",
        "sim::tick_delta",
        "physics::joint_handle",
        "physics::body_handle",
        "ecs::in_region",
        "ecs::claimed_by",
        "predict::staged_region",
        "sim::current_tick",
        "sim::region_policy",
    ];
    assert_eq!(named(false), uncovered);
    // 0x1458_0671_8FB1_1F86 before `lactose-present-api` joined the engine lists (D162).
    assert_eq!(ProtocolHash::of(registry), ProtocolHash(0xB439_7FA7_135F_B2DE));
}
