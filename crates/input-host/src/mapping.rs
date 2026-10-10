//! Action mapping (architecture section 4: "action mapping"; section 22, milestone 5): the
//! table from device controls to [`Action`](lactose_input::Action) bits and movement
//! axes, loaded as an asset and consulted by the [`sources`](crate::sources). Nothing is
//! here yet: the named actions of milestone 1 are fixed in `Action`, and the generic bits
//! above [`Action::NAMED`](lactose_input::Action::NAMED) are what a map assigns.
