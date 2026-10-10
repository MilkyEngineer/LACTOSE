//! Action mapping (architecture section 4: "action mapping"; section 22, milestone 5): the
//! table from device controls to [`Action`](crate::Action) bits and movement axes, loaded
//! as an asset and consulted by the [`sources`](crate::sources). Nothing is here yet: the
//! named actions of milestone 1 are fixed in [`Action`](crate::Action), and the generic
//! bits above [`Action::NAMED`](crate::Action::NAMED) are what a map assigns.
//!
//! Host-only. A boundary crate names nothing in this module.
