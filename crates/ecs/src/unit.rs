//! The per-chunk kernel path: a per-entity system runs as units, each one chunk or a range
//! of one chunk's rows, and its `each` entry point runs the kernel over a unit's rows with
//! every column the parameters name lent as a slice for the whole unit. What a parameter
//! needs once per unit (a column, a resource, an event list) is taken before the first row;
//! what changes per row (the row's values, its key, its region, its output sinks) is made
//! from the column and the [`Row`].
//!
//! A unit owns everything it writes: its rows' outputs, its reducer partial and its counts.
//! The run that made it merges those after every unit has run, by key, never in the order
//! the units ran.

use core::any::{Any, TypeId};
use core::marker::PhantomData;

use lactose_asset_api::PinnedTable;
use lactose_core::{StableKey, SystemKey, Tick};
use lactose_math::Reducer;

use crate::access::{Access, AccessTarget};
use crate::change::Stamp;
use crate::command::{ChannelCommand, ChannelStore, CommandBuffer};
use crate::event::{EventRecord, EventStore, PresentRecord};
use crate::param::SystemParam;
use crate::region::{ClaimedBy, InRegion, RegionId};
use crate::run::{Output, SinkCursor, Sinks};
use crate::schedule::Source;
use crate::storage::{typed_slice, typed_slice_mut};
use crate::system::mismatch;

/// What every unit of one system's run shares.
pub(crate) struct UnitEnv<'r> {
    pub(crate) table: &'static [Access],
    pub(crate) system: SystemKey,
    pub(crate) tick: Tick,
    /// The stamp a mutable dereference writes.
    pub(crate) now: Stamp,
    pub(crate) resources: &'r [(TypeId, &'r (dyn Any + Send + Sync))],
    pub(crate) events: &'r EventStore,
    pub(crate) channels: &'r ChannelStore,
    /// The regions whose rows run, ascending by ID.
    pub(crate) regions: Vec<Admitted<'r>>,
    pub(crate) sinks: Sinks,
}

/// A region whose rows run: simulated, and holding every region resource the system reads.
pub(crate) struct Admitted<'r> {
    pub(crate) id: RegionId,
    pub(crate) seed: u64,
    pub(crate) assets: Option<&'r PinnedTable>,
    /// The bytes of each region resource the system reads, in parameter order.
    pub(crate) resources: Vec<&'r [u8]>,
}

/// One parameter's column over a unit's rows, before the parameter takes it.
pub(crate) enum Lent<'r> {
    /// No column: not a component parameter, a filter, or already taken.
    None,
    /// An optional component the archetype lacks.
    Absent,
    Read(&'r [u8]),
    /// The values and, on a tracked column, the row stamps.
    Write(&'r mut [u8], Option<(&'r mut [Tick], &'r mut [u32])>),
}

/// The rows of one unit and what its entry point writes. Not part of the interface.
#[doc(hidden)]
pub struct Unit<'r> {
    env: &'r UnitEnv<'r>,
    keys: &'r [StableKey],
    regions: &'r [InRegion],
    claims: Option<&'r [ClaimedBy]>,
    /// The chunk's selection by its `Changed` filters, one bit per row, and the index of
    /// the unit's first row in it; `None` without a filter.
    selected: Option<(&'r [u64], usize)>,
    columns: Vec<Lent<'r>>,
    next: usize,
    /// The region of the last row looked up, and its index in `env.regions` if admitted.
    region: Option<(RegionId, Option<usize>)>,
    /// The output of the row handed out last, or one no row has written to.
    output: Output,
    /// Whether `output` belongs to the row handed out last.
    open: bool,
    pub(crate) outputs: Vec<Output>,
    pub(crate) partial: Option<Box<dyn Any + Send>>,
    /// The rows the kernel ran on.
    pub(crate) invocations: usize,
}

impl<'r> Unit<'r> {
    pub(crate) fn new(
        env: &'r UnitEnv<'r>,
        keys: &'r [StableKey],
        regions: &'r [InRegion],
        claims: Option<&'r [ClaimedBy]>,
        selected: Option<(&'r [u64], usize)>,
        columns: Vec<Lent<'r>>,
    ) -> Self {
        Self {
            env,
            keys,
            regions,
            claims,
            selected,
            columns,
            next: 0,
            region: None,
            output: Output::new(Source::Tick, None, None, env.sinks),
            open: false,
            outputs: Vec::new(),
            partial: None,
            invocations: 0,
        }
    }

    /// The rows of the unit, whether the kernel runs on them or not.
    pub(crate) fn len(&self) -> usize {
        self.keys.len()
    }

    pub(crate) fn env(&self) -> &'r UnitEnv<'r> {
        self.env
    }

    /// The column lent to parameter `index`, which takes it once.
    pub(crate) fn lend(&mut self, index: usize) -> Lent<'r> {
        core::mem::replace(&mut self.columns[index], Lent::None)
    }

    /// The table entry of parameter `index`.
    pub(crate) fn access(&self, index: usize) -> &'static Access {
        &self.env.table[index]
    }

    /// Closes the row handed out last, keeping its output if it wrote any, and hands out
    /// the next row the kernel runs on: in an admitted region and selected by every
    /// `Changed` filter.
    fn next_row(&mut self) -> Option<Row<'_>> {
        if core::mem::take(&mut self.open) {
            self.output.close_channels();
            if !self.output.is_empty() {
                let fresh = Output::new(Source::Tick, None, None, self.env.sinks);
                self.outputs.push(core::mem::replace(&mut self.output, fresh));
            }
        }
        while self.next < self.keys.len() {
            let index = self.next;
            self.next += 1;
            if let Some((bits, first)) = self.selected {
                let bit = first + index;
                if (bits[bit / 64] >> (bit % 64)) & 1 == 0 {
                    continue;
                }
            }
            let id = self.regions[index].region;
            let admitted = match self.region {
                Some((last, admitted)) if last == id => admitted,
                _ => {
                    let admitted = self.env.regions.binary_search_by_key(&id, |r| r.id).ok();
                    self.region = Some((id, admitted));
                    admitted
                }
            };
            let Some(admitted) = admitted else { continue };
            let key = self.keys[index];
            let claim = self.claims.map(|claims| claims[index].player);
            self.invocations += 1;
            self.open = true;
            self.output.open(Source::Entity(key), claim, Some(id));
            let env = self.env;
            let Output { commands, events, present, channels, .. } = &mut self.output;
            return Some(Row {
                index,
                key,
                region: &env.regions[admitted],
                env,
                commands: Some(commands),
                emit: SinkCursor::new(events),
                present: SinkCursor::new(present),
                channels: SinkCursor::new(channels),
            });
        }
        None
    }
}

/// One row of a unit, which the kernel runs on: what a parameter needs per row beside its
/// column. Not part of the interface.
#[doc(hidden)]
pub struct Row<'a> {
    /// The row's index in the unit's columns.
    pub(crate) index: usize,
    pub(crate) key: StableKey,
    pub(crate) region: &'a Admitted<'a>,
    pub(crate) env: &'a UnitEnv<'a>,
    pub(crate) commands: Option<&'a mut CommandBuffer>,
    pub(crate) emit: SinkCursor<'a, Vec<EventRecord>>,
    pub(crate) present: SinkCursor<'a, Vec<PresentRecord>>,
    pub(crate) channels: SinkCursor<'a, Vec<ChannelCommand>>,
}

/// What a parameter takes from a unit before its first row: its column, or what it reads
/// for every row. Not part of the interface.
#[doc(hidden)]
pub trait UnitColumn<'r>: Sized {
    /// The column of parameter `index`.
    fn take(unit: &mut Unit<'r>, index: usize) -> Self;
}

/// How a parameter is made for one row from what it took from the unit. Implemented for
/// exactly the [`SystemParam`] types; a kind no per-entity system may take has a column
/// that cannot be made. Not part of the interface.
#[doc(hidden)]
pub trait RowFetch<'a, 'r>: Sized {
    /// What the parameter takes from the unit.
    type Column: UnitColumn<'r>;

    /// The parameter for `row`.
    fn row(column: &'a mut Self::Column, row: &mut Row<'a>) -> Self;
}

/// The column of parameter `index`, for the expansions; its type is inferred from the
/// parameter the column later makes.
#[doc(hidden)]
pub fn column<'r, C: UnitColumn<'r>>(unit: &mut Unit<'r>, index: usize) -> C {
    C::take(unit, index)
}

/// The next row the kernel runs on, for the expansions.
#[doc(hidden)]
pub fn next_row<'u>(unit: &'u mut Unit<'_>) -> Option<Row<'u>> {
    unit.next_row()
}

/// The parameter made from `column` for `row`, for the expansions.
#[doc(hidden)]
pub fn row<'a, 'r, P: SystemParam + RowFetch<'a, 'r>>(
    column: &'a mut P::Column,
    row: &mut Row<'a>,
) -> P {
    P::row(column, row)
}

/// A unit's empty partial, for the expansion of a reduction.
#[doc(hidden)]
#[must_use]
pub fn identity<R: Reducer>() -> R {
    R::identity()
}

/// Folds one kernel result into the unit's partial, for the expansion of a reduction.
#[doc(hidden)]
pub fn add<R: Reducer>(partial: &mut R, item: R::Item) {
    partial.add(item);
}

/// Hands the unit's partial to the run, which merges every unit's, for the expansion of a
/// reduction.
#[doc(hidden)]
pub fn keep<R: Reducer + Send + 'static>(unit: &mut Unit<'_>, partial: R) {
    unit.partial = Some(Box::new(partial));
}

// The columns of the parameter kinds. Each is taken once per unit and makes its parameter
// for every row by the row's index.

/// The values of a component, read.
#[doc(hidden)]
pub struct ReadColumn<'r, T> {
    pub(crate) values: &'r [T],
}

/// The values of a component, written, with the row stamps of a tracked column.
#[doc(hidden)]
pub struct MutColumn<'r, T> {
    pub(crate) values: &'r mut [T],
    pub(crate) stamps: Option<(&'r mut [Tick], &'r mut [u32])>,
    pub(crate) now: Stamp,
}

/// Panics unless parameter `index`'s table entry targets `target`. The run lends each
/// parameter what its entry names, so a kernel whose code disagrees with its table panics
/// here instead of reading what was lent for that entry as another type.
fn expect_target(unit: &Unit<'_>, index: usize, target: AccessTarget) {
    if unit.access(index).target() != target {
        mismatch(index);
    }
}

impl<'r, T: lactose_reflect::Component> UnitColumn<'r> for ReadColumn<'r, T> {
    fn take(unit: &mut Unit<'r>, index: usize) -> Self {
        expect_target(unit, index, AccessTarget::Component(T::KEY));
        match unit.lend(index) {
            Lent::Read(bytes) => Self { values: typed_slice(bytes, unit.len()) },
            _ => mismatch(index),
        }
    }
}

impl<'r, T: lactose_reflect::Component> UnitColumn<'r> for Option<ReadColumn<'r, T>> {
    fn take(unit: &mut Unit<'r>, index: usize) -> Self {
        expect_target(unit, index, AccessTarget::Component(T::KEY));
        match unit.lend(index) {
            Lent::Read(bytes) => Some(ReadColumn { values: typed_slice(bytes, unit.len()) }),
            Lent::Absent => None,
            _ => mismatch(index),
        }
    }
}

fn mut_column<'r, T: lactose_reflect::Component>(
    unit: &Unit<'r>,
    bytes: &'r mut [u8],
    stamps: Option<(&'r mut [Tick], &'r mut [u32])>,
) -> MutColumn<'r, T> {
    MutColumn { values: typed_slice_mut(bytes, unit.len()), stamps, now: unit.env.now }
}

impl<'r, T: lactose_reflect::Component> UnitColumn<'r> for MutColumn<'r, T> {
    fn take(unit: &mut Unit<'r>, index: usize) -> Self {
        expect_target(unit, index, AccessTarget::Component(T::KEY));
        match unit.lend(index) {
            Lent::Write(bytes, stamps) => mut_column(unit, bytes, stamps),
            _ => mismatch(index),
        }
    }
}

impl<'r, T: lactose_reflect::Component> UnitColumn<'r> for Option<MutColumn<'r, T>> {
    fn take(unit: &mut Unit<'r>, index: usize) -> Self {
        expect_target(unit, index, AccessTarget::Component(T::KEY));
        match unit.lend(index) {
            Lent::Write(bytes, stamps) => Some(mut_column(unit, bytes, stamps)),
            Lent::Absent => None,
            _ => mismatch(index),
        }
    }
}

/// Nothing per unit: a filter, the row's key or its context.
#[doc(hidden)]
pub struct NoColumn;

impl<'r> UnitColumn<'r> for NoColumn {
    fn take(_: &mut Unit<'r>, _: usize) -> Self {
        Self
    }
}

/// The parameter's index, for the output sink of parameter type `P` the row hands out.
#[doc(hidden)]
pub struct SinkColumn<P>(pub(crate) usize, PhantomData<fn() -> P>);

impl<'r, P: SystemParam> UnitColumn<'r> for SinkColumn<P> {
    fn take(unit: &mut Unit<'r>, index: usize) -> Self {
        // The sinks are made per table entry, so the entry must be `P`'s own: its kind
        // tells an event sink from a presentation one, its target one event type from another.
        let access = unit.access(index);
        if access.kind() != P::ACCESS.kind() || access.target() != P::ACCESS.target() {
            mismatch(index);
        }
        Self(index, PhantomData)
    }
}

/// The place of region resource `T` among the system's, for the row's region.
#[doc(hidden)]
pub struct RegionResColumn<T>(pub(crate) usize, PhantomData<fn() -> T>);

impl<'r, T: lactose_reflect::Resource> UnitColumn<'r> for RegionResColumn<T> {
    fn take(unit: &mut Unit<'r>, index: usize) -> Self {
        expect_target(unit, index, AccessTarget::RegionResource(T::KEY));
        let before = unit.env.table[..index]
            .iter()
            .filter(|access| matches!(access.target(), AccessTarget::RegionResource(_)))
            .count();
        Self(before, PhantomData)
    }
}

/// The column of a parameter no per-entity system takes, which the schedule rejects: it
/// cannot be made, so its parameter is never made for a row.
#[doc(hidden)]
pub enum NeverColumn {}

impl<'r> UnitColumn<'r> for NeverColumn {
    fn take(_: &mut Unit<'r>, index: usize) -> Self {
        mismatch(index)
    }
}
