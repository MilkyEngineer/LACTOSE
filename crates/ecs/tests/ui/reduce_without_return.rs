use lactose_ecs::system;
use lactose_math::{ExactSum, Scalar};
use lactose_reflect::component;

#[component(key = "ui::mass", persistent)]
pub struct Mass(#[field(id = 1)] pub Scalar);

#[system(name = "ui::f", reduce = ExactSum<Scalar>)]
fn f(_mass: &Mass) {}

fn main() {}
