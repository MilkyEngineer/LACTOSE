use lactose_ecs::{finalize, system};
use lactose_math::{ExactSum, Scalar};
use lactose_reflect::component;

#[component(key = "ui::mass", persistent)]
pub struct Mass(#[field(id = 1)] pub Scalar);

#[system(name = "ui::total", reduce = ExactSum<Scalar>)]
fn total(mass: &Mass) -> Scalar {
    mass.0
}

#[finalize(total)]
fn store(_sum: u64) {}

fn main() {}
