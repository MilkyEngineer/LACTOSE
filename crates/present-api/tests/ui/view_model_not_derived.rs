//! A view-model that is not declared `derived` does not build: `ViewModels::set`, `get`
//! and `remove` each assert it where the call is instantiated, so this case is built, not
//! checked.

#![forbid(unsafe_code)]

use lactose_present_api::ViewModels;
use lactose_reflect::resource;

#[resource(key = "ui::saved_hud", persistent)]
pub struct SavedHud(#[field(id = 1)] pub u32);

fn main() {
    let mut table = ViewModels::new();
    table.set(SavedHud(1));
    let _ = table.get::<SavedHud>();
    let _ = table.remove::<SavedHud>();
}
