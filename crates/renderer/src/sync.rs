//! The atomics [`Staging`](crate::staging::Staging) shares between the writers and the
//! renderer: the standard library's, or loom's in the crate's own unit tests built with its
//! `loom` feature, so that the models in `models` run the shipped functions and not a copy
//! of them. The feature changes nothing the crate ships: only `cfg(test)` builds see loom.

#[cfg(all(test, feature = "loom"))]
pub(crate) use loom::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(all(test, feature = "loom")))]
pub(crate) use std::sync::atomic::{AtomicU64, Ordering};
