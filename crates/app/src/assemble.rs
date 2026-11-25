//! What every world kind is built from: the registration, the setups and the systems in the
//! order `README.md` states ("Registration"), the solver pool and the backend beside it.
//! Hidden plumbing shared by `build_server` and `build_client`.

use lactose_core::{Error, ErrorKind, Result};
use lactose_ecs::WorldKind;
use lactose_input::PlayerInputApplier;
use lactose_persist::SchemaTable;
use lactose_sim::{Registration, SimulationBuilder, TypeTable};
use lactose_task::Executor;

use crate::game::{Game, named};
use crate::runtime::{CATCH_UP_CAP, SolverExit, solver_pool};
use crate::world::WorldConfig;

/// The registry a world is built from and the schema table over it, computed once per world:
/// `Game::types`, then `SchemaTable::from_registry`.
pub(crate) fn registry(game: &Game) -> Result<(TypeTable, SchemaTable)> {
    let types = game.types()?;
    let schemas = SchemaTable::from_registry(types.registry())?;
    Ok((types, schemas))
}

/// The builder of a world of `kind` over `types`, with everything every kind gets, and the
/// exit count of its solver pool: `config` into the builder, the engine's setups, the
/// backend's with a pool of its own, every crate's setup, the engine's systems, every
/// crate's systems, the game's edges, the assembly's own edges, and the input applier.
/// What only one kind gets (the session applier, the start tick, the prediction check) is
/// the caller's. Refuses a `log_retention` below the cap before anything starts.
pub(crate) fn world(
    executor: &Executor,
    game: &Game,
    config: &WorldConfig,
    kind: WorldKind,
    types: TypeTable,
) -> Result<(SimulationBuilder, SolverExit)> {
    if config.log_retention < CATCH_UP_CAP {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "a log retention of {} ticks is below the catch-up cap of {CATCH_UP_CAP}",
                config.log_retention
            ),
        ));
    }
    let (pool, solver) = solver_pool(executor, config.physics.reserved_workers)?;
    let backend = lactose_physics_rapier::backend(pool);
    let physics = config.physics;
    let mut builder = SimulationBuilder::new(kind, types)
        .layout(config.layout)
        .regions(config.regions.clone())
        .policy(config.policy)
        .log_retention(config.log_retention)
        .setup(lactose_net_api::setup)
        .setup(move |world| lactose_physics_api::setup(world, backend, physics))
        .setup(lactose_present_api::setup);
    for registration in game.crates {
        if let Some(setup) = registration.setup {
            let name = registration.name;
            builder = builder.setup(move |world| setup(world).map_err(|error| named(name, &error)));
        }
    }
    builder = builder
        .systems(lactose_input::register_systems)
        .systems(lactose_net_api::register_systems)
        .systems(lactose_physics_api::register_systems)
        .systems(lactose_anim::register_systems)
        .systems(lactose_present_api::register_systems);
    for registration in game.crates {
        builder = builder.systems(registration.systems);
    }
    if let Some(order) = game.order {
        builder = builder.systems(order);
    }
    let builder = builder.systems(engine_order).inputs(Box::new(PlayerInputApplier::new()));
    Ok((builder, solver))
}

/// The assembly's own ordering edges between engine crates' sets: none at milestone 1, since
/// no two engine crates place an exclusive system in one stage or pass.
fn engine_order(_: &mut Registration) {}
