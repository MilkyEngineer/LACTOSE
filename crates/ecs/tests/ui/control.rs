//! The control: systems of every kind, a reduction with its finalize and an exclusive
//! system, in a crate that forbids `unsafe` and denies warnings, so a failure of the other
//! cases is known to be the declaration's and not the harness's.

#![forbid(unsafe_code)]
#![deny(warnings)]

use lactose_ecs::{
    Commands, Ctx, Job, Mut, Res, ResMut, StableKeyOf, SystemSet, World, finalize, system,
};
use lactose_math::{ExactSum, Scalar};
use lactose_reflect::{component, resource};

#[component(key = "ui::mass", persistent)]
pub struct Mass(#[field(id = 1)] pub Scalar);

#[resource(key = "ui::total", persistent)]
pub struct Total(#[field(id = 1)] pub Scalar);

pub static SET: SystemSet = SystemSet::new("ui::set");

#[system(name = "ui::per_entity", worlds = [server, client, prediction], in_set = SET)]
fn per_entity(key: StableKeyOf, mass: Mut<Mass>, cmd: Commands) {
    mass.0 = Scalar::ONE;
    cmd.despawn(*key);
}

#[system(name = "ui::per_region", after = [per_entity], writes = [Mass])]
fn per_region(ctx: Ctx, cmd: Commands) {
    cmd.spawn((Mass(Scalar::from_u32(ctx.tick().0)),));
}

#[system(name = "ui::once", worlds = [server])]
fn once(job: Job, total: ResMut<Total>) {
    let _ = job.context().scratch();
    total.0 = Scalar::ZERO;
}

#[system(name = "ui::sum", reduce = ExactSum<Scalar>)]
fn sum(mass: &Mass) -> Scalar {
    mass.0
}

#[finalize(sum)]
fn store(sum: Scalar, total: ResMut<Total>) {
    total.0 = sum;
}

#[system(name = "ui::exclusive", exclusive)]
fn exclusive(world: &mut World, _total: Res<Total>) {
    let _ = world.tick();
}

fn main() {
    let names = [&PER_ENTITY, &PER_REGION, &ONCE, &SUM, &EXCLUSIVE].map(|system| system.name());
    assert_eq!(names.len(), 5);
}
