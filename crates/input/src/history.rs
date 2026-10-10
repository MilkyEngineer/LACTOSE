//! The redundancy window (architecture section 9.2): the last [`REDUNDANT_SAMPLES`] ticks'
//! samples, which the client keeps and packs into every input packet and the server fills
//! from every packet it receives, so a single lost packet costs nothing.
//!
//! What boundary code may use, though no boundary crate needs it at milestone 1: integers
//! and ticks only. Substitution of a sample still missing after redundancy is
//! [`PlayerInput::substitute`], a pure function; the jitter window and when to give up on
//! a tick are `net`'s (milestone 4).

use lactose_core::Tick;

use crate::sample::PlayerInput;

/// How many ticks' samples a packet carries and a [`SampleHistory`] holds: eight, 133 ms
/// at 60 Hz, so at a 30 Hz send rate three consecutive lost packets lose no sample. `net`
/// may amend it with its bandwidth model; the window is not part of any schema.
pub const REDUNDANT_SAMPLES: usize = 8;

/// The samples of the [`CAPACITY`](Self::CAPACITY) ticks up to and including the newest
/// inserted, one slot per tick, a slot possibly empty: on the client every slot is filled
/// in tick order; on the server a packet's window is inserted entry by entry, in any
/// order, and a slot stays empty for a tick no packet delivered.
///
/// Ticks are compared by wrapping difference (`tick.0.wrapping_sub(newest.0) as i32`), so
/// the window is correct across the `u32` wrap as long as no two ticks in play are more
/// than 2^31 apart.
#[derive(Clone, Debug)]
pub struct SampleHistory {
    slots: [Option<PlayerInput>; REDUNDANT_SAMPLES],
    newest: Option<Tick>,
}

impl SampleHistory {
    /// The number of ticks the window spans.
    pub const CAPACITY: usize = REDUNDANT_SAMPLES;

    /// An empty history.
    #[must_use]
    pub const fn new() -> Self {
        Self { slots: [None; REDUNDANT_SAMPLES], newest: None }
    }

    /// Stores `input` as the sample of `tick` and reports whether it was stored. A tick
    /// newer than the newest held moves the window forward to it, dropping the slots that
    /// fall out; a tick inside the window fills its slot if empty and is ignored (returning
    /// `false`) if the slot holds a sample already, so the first delivery of a tick wins;
    /// a tick older than the window is ignored.
    pub fn insert(&mut self, tick: Tick, input: PlayerInput) -> bool {
        todo!()
    }

    /// The sample of `tick`, if the window holds it.
    #[must_use]
    pub fn get(&self, tick: Tick) -> Option<&PlayerInput> {
        todo!()
    }

    /// The newest tick inserted and its sample.
    #[must_use]
    pub fn newest(&self) -> Option<(Tick, &PlayerInput)> {
        todo!()
    }

    /// Every held sample, in ascending tick order: what the client packs into a packet.
    #[must_use]
    pub fn iter(&self) -> SampleIter<'_> {
        todo!()
    }

    /// How many slots hold a sample.
    #[must_use]
    pub fn len(&self) -> usize {
        todo!()
    }

    /// Whether no slot holds a sample.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        todo!()
    }
}

impl Default for SampleHistory {
    fn default() -> Self {
        Self::new()
    }
}

/// The held samples of a [`SampleHistory`] in ascending tick order, from
/// [`SampleHistory::iter`].
#[derive(Clone, Debug)]
pub struct SampleIter<'a> {
    history: &'a SampleHistory,
    next: usize,
}

impl<'a> Iterator for SampleIter<'a> {
    type Item = (Tick, &'a PlayerInput);

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

impl<'a> IntoIterator for &'a SampleHistory {
    type Item = (Tick, &'a PlayerInput);
    type IntoIter = SampleIter<'a>;

    fn into_iter(self) -> SampleIter<'a> {
        self.iter()
    }
}
