//! Architecture section 6.1, as written, in a crate that forbids unsafe and denies warnings.
#![forbid(unsafe_code)]
#![deny(warnings)]

use lactose_core::{EntityRef, StableKey};
use lactose_ecs::{
    Commands, CommandChannel, Ctx, Event, EventWriter, Events, Mut, PresentEvent, PresentEvents,
    PresentKind, Res, ResMut, StableKeyOf, SystemSet, World, finalize, system,
};
use lactose_math::{ExactSum, Scalar, Vec2};
use lactose_reflect::{component, resource, value, Padding};

#[component(key = "g::velocity", persistent)]
pub struct Velocity { #[field(id = 1)] pub x: Scalar, #[field(id = 2)] pub y: Scalar }
#[component(key = "g::mass", persistent)]
pub struct Mass(#[field(id = 1)] pub Scalar);
#[component(key = "g::bomb", persistent)]
pub struct Bomb { #[field(id = 1)] pub fuse: u32 }
#[component(key = "g::thrown_by", persistent)]
pub struct ThrownBy { #[field(id = 1)] pub thrower: EntityRef }
#[value]
#[repr(u8)]
#[derive(PartialEq, Eq)]
pub enum ItemKind { None = 0, Bomb = 1 }
#[component(key = "g::held_item", persistent)]
pub struct HeldItem { #[field(id = 1)] pub kind: ItemKind }
#[component(key = "g::player_input", transient)]
pub struct PlayerInput { #[field(id = 1)] pub aim: Vec2, #[field(id = 2)] pub throw: bool, #[field(padding)] pub _pad: Padding<7> }
#[component(key = "g::health", persistent, replicated)]
pub struct Health(#[field(id = 1)] pub i32);
#[value]
pub struct Hit { #[field(id = 1)] pub key: EntityRef, #[field(id = 2)] pub amount: i32 }
#[component(key = "g::hit_list", derived)]
pub struct HitList { #[field(id = 1)] pub hits: lactose_core::PlainArray<Hit, 2>, #[field(id = 2)] pub len: u32 }
impl HitList { pub fn iter(&self) -> impl Iterator<Item = &Hit> { self.hits.iter().take(self.len as usize) } }
#[component(key = "g::anim", persistent)]
pub struct AnimState { #[field(id = 1)] pub phase: u32 }
impl AnimState { pub fn phase_crossed(&self, x: u32) -> bool { self.phase == x } }

#[resource(key = "g::tick_delta", persistent)]
pub struct TickDelta(#[field(id = 1)] pub Scalar);
#[resource(key = "g::gravity", persistent)]
pub struct Gravity(#[field(id = 1)] pub Scalar);
#[resource(key = "g::total_mass", persistent)]
pub struct TotalMass(#[field(id = 1)] pub Scalar);
#[resource(key = "g::loaded", persistent)]
pub struct LoadedBundles(#[field(id = 1)] pub u32);

#[derive(Clone, Copy)]
pub struct Damage { pub target: StableKey, pub source: StableKey, pub amount: i32 }
impl Event for Damage { fn target(&self) -> StableKey { self.target } }

#[derive(Clone, Copy, bytemuck::NoUninit, bytemuck::CheckedBitPattern)]
#[repr(u32)]
pub enum AudioRequest { Footstep = 1 }
impl PresentEvent for AudioRequest { const KIND: PresentKind = PresentKind::from_name("g::audio"); }

pub struct PhysicsChannel;
pub struct Impulse { pub body: StableKey, pub impulse: Vec2 }
impl CommandChannel for PhysicsChannel {
    type Entry = Impulse;
    fn remap(entry: &mut Impulse, f: &mut dyn FnMut(&mut StableKey)) { f(&mut entry.body); }
}
pub struct PhysicsWriter<'a, 'w>(lactose_ecs::ChannelWriter<'a, PhysicsChannel>, core::marker::PhantomData<&'w ()>);
impl PhysicsWriter<'_, '_> { pub fn apply_impulse(&mut self, body: StableKey, impulse: Vec2) { self.0.push(Impulse { body, impulse }); } }
pub trait PhysicsExt<'w> { fn physics(&mut self) -> PhysicsWriter<'_, 'w>; }
impl<'w> PhysicsExt<'w> for Commands<'w> {
    fn physics(&mut self) -> PhysicsWriter<'_, 'w> { PhysicsWriter(self.channel::<PhysicsChannel>(), core::marker::PhantomData) }
}

pub mod sets {
    pub static MOVEMENT: lactose_ecs::SystemSet = lactose_ecs::SystemSet::new("game_sim::movement");
}
use sets::MOVEMENT;

// Per-entity kernel.
#[system(name = "game_sim::apply_gravity", worlds = [server, prediction], in_set = MOVEMENT)]
fn apply_gravity(dt: Res<TickDelta>, g: Res<Gravity>, vel: Mut<Velocity>, mass: &Mass) {
    let _ = mass;
    vel.y -= g.0 * dt.0;
}

#[system(name = "game_sim::total_mass", worlds = [server], reduce = ExactSum<Scalar>)]
fn total_mass(mass: &Mass) -> Scalar { mass.0 }
#[finalize(total_mass)]
fn store_total(sum: Scalar, out: ResMut<TotalMass>) { out.0 = sum; }

#[system(name = "game_sim::throw_bomb", worlds = [server, prediction])]
fn throw_bomb(ctx: Ctx, cmd: Commands, key: StableKeyOf, held: &HeldItem, input: &PlayerInput) {
    if input.throw && held.kind == ItemKind::Bomb {
        let bomb = cmd.spawn((Bomb { fuse: 180 },));
        cmd.set(*key, HeldItem { kind: ItemKind::None });
        cmd.set(bomb, ThrownBy { thrower: EntityRef::new(*key) });
        let spread = ctx.rng(bomb).next_scalar();
        cmd.physics().apply_impulse(bomb, Vec2::new(input.aim.x * spread, input.aim.y * spread));
    }
}

#[system(name = "game_sim::sword_hit", worlds = [server, prediction])]
fn sword_hit(damage: EventWriter<Damage>, hits: &HitList, key: StableKeyOf) {
    for h in hits.iter() { if let Some(t) = h.key.get() { damage.emit(Damage { target: t, source: *key, amount: h.amount }); } }
}
#[system(name = "game_sim::apply_damage", worlds = [server, prediction], after = sword_hit)]
fn apply_damage(events: Events<Damage>, key: StableKeyOf, hp: Mut<Health>) {
    for d in events.for_target(*key) { hp.0 -= d.amount; }
}

#[system(name = "game_sim::footsteps", worlds = [server, prediction])]
fn footsteps(present: PresentEvents, key: StableKeyOf, anim: &AnimState) {
    if anim.phase_crossed(8) { present.emit(*key, AudioRequest::Footstep); }
}

#[system(name = "sim::apply_region_load", exclusive)]
fn apply_region_load(world: &mut World, loaded: ResMut<LoadedBundles>) { let _ = (world.tick(), loaded.0); }

// After a system in another module, and a client-only system.
mod other {
    use lactose_ecs::{Res, system};
    #[system(name = "other::late", after = super::apply_gravity, worlds = [client])]
    pub(crate) fn late(_g: Res<super::Gravity>) {}
}

fn main() {
    let _ = &SystemSet::new("x");
    let all = [&APPLY_GRAVITY, &TOTAL_MASS, &THROW_BOMB, &SWORD_HIT, &APPLY_DAMAGE, &FOOTSTEPS, &APPLY_REGION_LOAD, &other::LATE];
    assert_eq!(all.len(), 8);
}
