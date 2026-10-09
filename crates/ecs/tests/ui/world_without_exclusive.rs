use lactose_ecs::{World, system};

#[system(name = "ui::f")]
fn f(_world: &mut World) {}

fn main() {}
