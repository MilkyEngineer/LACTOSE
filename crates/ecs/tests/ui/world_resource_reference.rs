//! `World::insert_resource` of a resource type that holds an entity reference does not
//! build (D142): in a field of its own, in a nested `#[value]` struct and in an array. The
//! assertion is evaluated where each call is instantiated, so this case is built, not
//! checked.

#![forbid(unsafe_code)]

use std::sync::Arc;

use lactose_core::{EntityRef, PlainArray};
use lactose_ecs::{World, WorldBuilder, WorldKind};
use lactose_reflect::{Registry, resource, value};

#[resource(key = "ui::target", derived)]
pub struct Target {
    #[field(id = 1)]
    pub entity: EntityRef,
}

#[value]
pub struct Aim {
    #[field(id = 1)]
    pub at: EntityRef,
}

#[resource(key = "ui::nested_target", derived)]
pub struct NestedTarget {
    #[field(id = 1)]
    pub weight: u64,
    #[field(id = 2)]
    pub aim: Aim,
}

#[resource(key = "ui::target_list", derived)]
pub struct TargetList {
    #[field(id = 1)]
    pub all: PlainArray<EntityRef, 2>,
}

fn insert(world: &mut World) {
    world.insert_resource(Target { entity: EntityRef::NONE });
    world.insert_resource(NestedTarget { weight: 1, aim: Aim { at: EntityRef::NONE } });
    world.insert_resource(TargetList { all: PlainArray::from([EntityRef::NONE; 2]) });
}

fn main() {
    if let Ok(mut world) = WorldBuilder::new(WorldKind::Server, Arc::new(Registry::new())).build() {
        insert(&mut world);
    }
}
