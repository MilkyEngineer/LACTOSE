//! Action mapping (architecture section 4: "action mapping"; section 22, milestone 5): the
//! table from device controls to [`Action`](lactose_input::Action) bits and movement
//! axes, loaded as an asset and consulted by the [`sources`](crate::sources). Nothing is
//! here yet: the named actions of milestone 1 are fixed in `Action`, and the game's bits,
//! from [`Actions::GAME_FIRST`](lactose_input::Actions::GAME_FIRST) up, are what a map
//! assigns.
