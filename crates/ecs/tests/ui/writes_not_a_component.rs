use lactose_ecs::{Commands, system};

#[system(name = "ui::f", writes = [u32])]
fn f(_cmd: Commands) {}

fn main() {}
