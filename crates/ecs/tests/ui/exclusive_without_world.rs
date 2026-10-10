use lactose_ecs::{Commands, system};

#[system(name = "ui::f", exclusive)]
fn f(_cmd: Commands) {}

fn main() {}
