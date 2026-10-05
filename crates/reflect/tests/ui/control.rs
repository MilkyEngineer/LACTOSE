//! The control: declarations of every form that compile in a crate which forbids `unsafe`
//! and depends on `lactose-reflect` alone, so a failure of the other cases is known to be
//! the declaration's and not the harness's.

#![forbid(unsafe_code)]
#![deny(warnings)]

use lactose_reflect::{Padding, Reflect, Registry, component, resource, value};

#[value]
#[repr(u8)]
pub enum Mode {
    Off = 0,
    On = 1,
}

#[value(retired = [2])]
pub struct Pair(#[field(id = 1)] pub u16, #[field(id = 3)] pub u16);

#[component(key = "ui::control", persistent, version = 2, replicated, retired = [4])]
pub struct Control {
    #[field(id = 1)]
    pub count: u32,
    #[field(id = 2)]
    pub pair: Pair,
    #[field(id = 3)]
    pub mode: Mode,
    #[field(padding)]
    _pad: Padding<3>,
}

#[component(key = "ui::tag", transient)]
pub struct Tag;

#[resource(key = "ui::gravity", persistent)]
pub struct Gravity(#[field(id = 1)] pub i64);

fn main() {
    let control = Control { count: 0, pair: Pair(1, 2), mode: Mode::On, _pad: Padding::default() };
    assert_eq!(Control::SCHEMA.layout().size(), size_of_val(&control));
    let mut registry = Registry::new();
    registry.register::<Control>().unwrap();
    registry.register::<Tag>().unwrap();
    registry.register::<Gravity>().unwrap();
}
