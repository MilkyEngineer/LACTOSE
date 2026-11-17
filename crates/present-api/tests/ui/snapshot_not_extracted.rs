//! A snapshot read of a component that is not declared `extracted` does not build:
//! `PresentSnapshot::get` and `rows` each assert it where the call is instantiated, so this
//! case is built, not checked.

#![forbid(unsafe_code)]

use lactose_core::{NetId, StableKey};
use lactose_present_api::{PresentSnapshot, snapshot_channel};
use lactose_reflect::component;

#[component(key = "ui::hidden", persistent)]
pub struct Hidden(#[field(id = 1)] pub u32);

fn read(snapshot: &PresentSnapshot) -> usize {
    let one = snapshot.get::<Hidden>(StableKey::Net(NetId::cooked(1)));
    usize::from(one.is_some()) + snapshot.rows::<Hidden>().count()
}

fn main() {
    let (_writer, mut reader) = snapshot_channel();
    if let Some(frame) = reader.acquire() {
        let _ = read(frame.newer());
    }
}
