//! The inside half of the game (architecture sections 3 and 8.1): gameplay that every world
//! kind registers and runs, under every rule of section 8.2. At milestone 2 it holds the
//! sketch game the headless binary runs, moved out of the binary so that what a world
//! registers is declared inside the boundary and the recorded test runs the system that
//! ships (D152; milestone 2 plan, decision 5.18). The game arrives at milestone 5 (section
//! 22).
//!
//! A game crate may name neither `lactose-app` nor `lactose-persist` (`crate-tiers.toml`),
//! so this crate exposes the functions a registration is built from and the binary holds the
//! `CrateRegistration` and `Game` values.

#![forbid(unsafe_code)]

mod sketch;

pub use sketch::{Counter, Orbit, register_systems, register_types, sessions};
