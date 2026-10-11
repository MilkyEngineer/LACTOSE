//! The redundancy window (architecture section 9.2): the last [`REDUNDANT_SAMPLES`] ticks'
//! samples, which the client keeps and packs into every input packet and the server fills
//! from every packet it receives, so a single lost packet costs nothing.
//!
//! Integers and ticks only. Substitution of a sample still missing after redundancy is
//! [`PlayerInput::substitute`], a pure function; the jitter window, when a tick counts as
//! lost and a deeper server-side buffer are `net`'s (milestone 4).

use lactose_core::Tick;

use crate::sample::PlayerInput;

/// How many ticks' samples a packet carries and a [`SampleHistory`] holds: eight, 133 ms
/// at 60 Hz, so at a 30 Hz send rate three consecutive lost packets lose no sample. `net`
/// may amend it with its bandwidth model; the window is not part of any schema.
pub const REDUNDANT_SAMPLES: usize = 8;

/// The samples of the [`CAPACITY`](Self::CAPACITY) ticks up to and including the newest
/// inserted, one slot per tick, a slot possibly empty: on the client every slot is filled
/// in tick order through [`insert`](Self::insert); on the server a packet's window is
/// inserted entry by entry, in any order, through [`insert_bounded`](Self::insert_bounded)
/// so that an entry off the wire cannot move the window away from the ticks still to be
/// applied, and a slot stays empty for a tick no packet delivered.
///
/// Ticks are compared by wrapping difference (`tick.0.wrapping_sub(other.0) as i32`), so
/// the window is correct across the `u32` wrap as long as no two ticks in play are more
/// than 2^31 apart, the ECS's own rule.
#[derive(Clone, Debug)]
pub struct SampleHistory {
    /// A ring: the slot of the tick `age` ticks older than `newest` is
    /// `(head + CAPACITY - age) % CAPACITY`, so the window moves without shifting and the
    /// slot of a tick never depends on its absolute value, which keeps the ring correct
    /// across the `u32` wrap whatever `CAPACITY` is.
    slots: [Option<PlayerInput>; REDUNDANT_SAMPLES],
    /// The slot of `newest`.
    head: usize,
    /// The newest tick inserted; its slot is never empty.
    newest: Option<Tick>,
}

impl SampleHistory {
    /// The number of ticks the window spans.
    pub const CAPACITY: usize = REDUNDANT_SAMPLES;

    /// An empty history.
    #[must_use]
    pub const fn new() -> Self {
        Self { slots: [None; REDUNDANT_SAMPLES], head: 0, newest: None }
    }

    /// Stores `input` as the sample of `tick` and reports whether it was stored. A tick
    /// newer than the newest held moves the window forward to it, dropping the slots that
    /// fall out; a tick inside the window fills its slot if empty and is ignored (returning
    /// `false`) if the slot holds a sample already, so the first delivery of a tick wins;
    /// a tick older than the window is ignored. The client's path, one call per tick.
    pub fn insert(&mut self, tick: Tick, input: PlayerInput) -> bool {
        let Some(newest) = self.newest else {
            self.newest = Some(tick);
            self.slots[self.head] = Some(input);
            return true;
        };
        let ahead = difference(tick, newest);
        if ahead > 0 {
            let ahead = ahead.unsigned_abs() as usize;
            for step in 1..=ahead.min(Self::CAPACITY) {
                self.slots[(self.head + step) % Self::CAPACITY] = None;
            }
            self.head = (self.head + ahead % Self::CAPACITY) % Self::CAPACITY;
            self.newest = Some(tick);
            self.slots[self.head] = Some(input);
            return true;
        }
        match self.slot(tick) {
            Some(slot) if self.slots[slot].is_none() => {
                self.slots[slot] = Some(input);
                true
            }
            _ => false,
        }
    }

    /// [`insert`](Self::insert) restricted to the ticks the server can still apply: `tick`
    /// is stored only if it lies in `(consumed, consumed + CAPACITY]` by wrapping
    /// difference, where `consumed` is the last tick applied for the player; otherwise
    /// `false` and nothing changes. The server's path for every entry of every packet: a
    /// forged or stale tick can neither move the window away from the ticks still to come
    /// nor resurrect one already applied. `net` keeps the consumed tick, and may keep a
    /// buffer of its own depth in front of this one.
    pub fn insert_bounded(&mut self, consumed: Tick, tick: Tick, input: PlayerInput) -> bool {
        let ahead = difference(tick, consumed);
        ahead >= 1 && ahead.unsigned_abs() as usize <= Self::CAPACITY && self.insert(tick, input)
    }

    /// The sample of `tick`, if the window holds it.
    #[must_use]
    pub fn get(&self, tick: Tick) -> Option<&PlayerInput> {
        self.slots[self.slot(tick)?].as_ref()
    }

    /// The newest tick inserted and its sample.
    #[must_use]
    pub fn newest(&self) -> Option<(Tick, &PlayerInput)> {
        Some((self.newest?, self.slots[self.head].as_ref()?))
    }

    /// Every held sample, in ascending tick order: what the client packs into a packet.
    #[must_use]
    pub fn iter(&self) -> SampleIter<'_> {
        SampleIter { history: self, next: 0 }
    }

    /// How many slots hold a sample.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|slot| slot.is_some()).count()
    }

    /// Whether no slot holds a sample.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.newest.is_none()
    }

    /// The slot of `tick`, if the window spans it.
    fn slot(&self, tick: Tick) -> Option<usize> {
        let age = difference(self.newest?, tick);
        let age = usize::try_from(age).ok().filter(|&age| age < Self::CAPACITY)?;
        Some((self.head + Self::CAPACITY - age) % Self::CAPACITY)
    }
}

/// How far `tick` is after `other`, by wrapping difference: negative when it is before.
fn difference(tick: Tick, other: Tick) -> i32 {
    tick.0.wrapping_sub(other.0) as i32
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
    /// How many of the window's ticks, oldest first, have been visited.
    next: usize,
}

impl<'a> Iterator for SampleIter<'a> {
    type Item = (Tick, &'a PlayerInput);

    fn next(&mut self) -> Option<Self::Item> {
        let newest = self.history.newest?;
        while self.next < SampleHistory::CAPACITY {
            let age = SampleHistory::CAPACITY - 1 - self.next;
            self.next += 1;
            let slot =
                (self.history.head + SampleHistory::CAPACITY - age) % SampleHistory::CAPACITY;
            if let Some(input) = &self.history.slots[slot] {
                // `age < CAPACITY`, far below `u32::MAX`.
                return Some((Tick(newest.0.wrapping_sub(age as u32)), input));
            }
        }
        None
    }
}

impl<'a> IntoIterator for &'a SampleHistory {
    type Item = (Tick, &'a PlayerInput);
    type IntoIter = SampleIter<'a>;

    fn into_iter(self) -> SampleIter<'a> {
        self.iter()
    }
}
