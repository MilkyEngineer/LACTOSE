//! Turns one environment variable into a `cfg`, so a Miri run can choose the real deque
//! without a Cargo feature that a normal build could switch on by accident.
//!
//! `LACTOSE_MIRI_REAL_DEQUE=1` makes the crate use `crossbeam-deque` under Miri instead of
//! its stand-in (`src/deque.rs`). It has no effect outside Miri.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=LACTOSE_MIRI_REAL_DEQUE");
    println!("cargo::rustc-check-cfg=cfg(lactose_miri_real_deque)");
    let wanted = std::env::var_os("LACTOSE_MIRI_REAL_DEQUE");
    if wanted.is_some_and(|value| !value.is_empty() && value != "0") {
        println!("cargo::rustc-cfg=lactose_miri_real_deque");
    }
}
