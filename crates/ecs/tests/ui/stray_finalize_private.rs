#![forbid(unsafe_code)]
#![deny(warnings)]
use lactose_ecs::{ResMut, finalize, system};
use lactose_math::Scalar;
use lactose_reflect::{component, resource};
#[component(key = "s::mass", persistent)]
pub struct Mass(#[field(id = 1)] pub Scalar);
#[resource(key = "s::total", persistent)]
pub struct Total(#[field(id = 1)] pub Scalar);
// A system that is not a reduction, with a finalize naming it: the finalize never runs.
#[system(name = "s::not_a_reduction")]
fn not_a_reduction(_m: &Mass) {}
#[finalize(not_a_reduction)]
fn store(sum: Scalar, out: ResMut<Total>) { out.0 = sum; }
// A finalize naming nothing at all.
#[finalize(nothing_here)]
fn store2(sum: Scalar, out: ResMut<Total>) { out.0 = sum; }
fn main() { let _ = &NOT_A_REDUCTION; }
