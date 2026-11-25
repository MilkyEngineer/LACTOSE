//! The shown frame run (`README.md`, "Proof for the shown frame", what `frame-impl`
//! proves) over a render side and a present side with no device, which log their calls and
//! answer as each test scripts. Each file is one part of the contract: the three states and
//! their resets, the waits and the bound of a paced frame, the protocol on each queue
//! layout, the shedding rule, and the ends of a run. Criterion 3's (a) and (b) are in
//! `shown`, section 5, over the same support.

pub mod support;

mod ends;
mod pacing;
mod protocol;
mod shedding;
mod states;
