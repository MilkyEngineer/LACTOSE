//! What the build cases get wrong, done right: a `derived` view-model through every method
//! of the table, and an `extracted` component read from a snapshot.

#![forbid(unsafe_code)]

use lactose_core::{NetId, StableKey};
use lactose_present_api::{PresentSnapshot, ViewModels, snapshot_channel};
use lactose_reflect::{component, resource};

#[resource(key = "ui::hud", derived)]
pub struct Hud(#[field(id = 1)] pub u32);

#[component(key = "ui::badge", persistent, extracted)]
pub struct Badge(#[field(id = 1)] pub u32);

fn read(snapshot: &PresentSnapshot) -> usize {
    let one = snapshot.get::<Badge>(StableKey::Net(NetId::cooked(1)));
    usize::from(one.is_some()) + snapshot.rows::<Badge>().count()
}

fn main() {
    let mut table = ViewModels::new();
    table.set(Hud(1));
    let _ = table.get::<Hud>();
    let _ = table.remove::<Hud>();
    let (_writer, mut reader) = snapshot_channel();
    if let Some(frame) = reader.acquire() {
        let _ = read(frame.newer());
    }
}
