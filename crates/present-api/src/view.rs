//! UI view-model resources (architecture section 13): the values a user interface shows,
//! kept on the render side. A view-model is a reflected resource declared
//! `#[resource(key = "...", derived)]`; this module is the table the presentation schedule
//! holds one of each in. `game-present` and `lactose-presentation` write it every frame
//! from the presentation snapshot and the frame's events, and `lactose-ui` reads it, by
//! type where it knows the type and by key and field ID where a cooked layout names them.
//!
//! No world holds a view-model and no system writes one: what the simulation tells the UI
//! reaches it as extracted components and presentation events, which `game-present` turns
//! into view-models (`README.md`, "View-models").

use std::any::TypeId;
use std::collections::BTreeMap;
use std::collections::btree_map;

use lactose_core::ComponentKey;
use lactose_reflect::{Persistence, Reflect, Resource, TypeInfo, bytemuck};

/// One stored view-model: its description, its type, and its bytes.
#[derive(Clone, Debug)]
struct Stored {
    info: &'static TypeInfo,
    type_id: TypeId,
    bytes: Box<[u8]>,
}

/// The view-models of one presentation schedule: at most one value of each type, by the
/// type's key.
///
/// A view-model type is declared `derived`, which every method here that takes the type
/// checks at build: it is never hashed, saved or replicated, it may hold a raw `f32` (a
/// screen position, an opacity), and a crate outside the determinism boundary that carries
/// no lint file may declare it (architecture section 8.1). The assertion is evaluated
/// where the call is instantiated, so a build reports it and `cargo check` does not.
#[derive(Clone, Debug, Default)]
pub struct ViewModels {
    values: BTreeMap<ComponentKey, Stored>,
}

impl ViewModels {
    /// An empty table.
    #[must_use]
    pub const fn new() -> Self {
        Self { values: BTreeMap::new() }
    }

    /// Stores `value` as the view-model of type `R`, returning the one it replaces.
    ///
    /// # Panics
    ///
    /// If the table holds a value of another type under `R::KEY`: two declared names
    /// whose keys collide, which no registry was asked about.
    pub fn set<R: Resource>(&mut self, value: R) -> Option<R> {
        const { assert!(is_derived::<R>(), "a view-model is declared `derived`") };
        if let Some(stored) = self.values.get(&R::KEY) {
            assert!(
                stored.type_id == TypeId::of::<R>(),
                "view-models `{}` and `{}` have one key",
                stored.info.name(),
                R::SCHEMA.info().name()
            );
        }
        let stored = Stored {
            info: R::SCHEMA.info(),
            type_id: TypeId::of::<R>(),
            bytes: bytemuck::bytes_of(&value).into(),
        };
        self.values.insert(R::KEY, stored).as_ref().map(read)
    }

    /// The view-model of type `R`, by value, or `None` if none is stored.
    #[must_use]
    pub fn get<R: Resource>(&self) -> Option<R> {
        const { assert!(is_derived::<R>(), "a view-model is declared `derived`") };
        self.values.get(&R::KEY).filter(|stored| stored.type_id == TypeId::of::<R>()).map(read)
    }

    /// Removes and returns the view-model of type `R`.
    pub fn remove<R: Resource>(&mut self) -> Option<R> {
        const { assert!(is_derived::<R>(), "a view-model is declared `derived`") };
        let stored = self.values.get(&R::KEY)?;
        if stored.type_id != TypeId::of::<R>() {
            return None;
        }
        self.values.remove(&R::KEY).as_ref().map(read)
    }

    /// The view-model whose type has `key`, as its description and bytes, or `None` if
    /// none is stored: how a reader that cannot name the type binds to a field.
    #[must_use]
    pub fn get_erased(&self, key: ComponentKey) -> Option<ViewModel<'_>> {
        self.values.get(&key).map(|stored| ViewModel { info: stored.info, bytes: &stored.bytes })
    }

    /// Every stored view-model, in ascending key order.
    #[must_use]
    pub fn iter(&self) -> ViewModelIter<'_> {
        ViewModelIter { values: self.values.values() }
    }

    /// How many view-models are stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether no view-model is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

impl<'a> IntoIterator for &'a ViewModels {
    type Item = ViewModel<'a>;
    type IntoIter = ViewModelIter<'a>;

    fn into_iter(self) -> ViewModelIter<'a> {
        self.iter()
    }
}

/// The value `stored` holds, which `set` copied from an `R`: a failure is a broken invariant.
fn read<R: Resource>(stored: &Stored) -> R {
    bytemuck::checked::try_pod_read_unaligned(&stored.bytes).expect("a stored view-model is valid")
}

/// Whether `R` is declared `derived`.
const fn is_derived<R: Reflect>() -> bool {
    matches!(R::SCHEMA.info().persistence(), Persistence::Derived)
}

/// One stored view-model, for a reader that cannot name its type.
#[derive(Clone, Copy, Debug)]
pub struct ViewModel<'a> {
    info: &'static TypeInfo,
    bytes: &'a [u8],
}

impl<'a> ViewModel<'a> {
    /// The type's description: its key, its name and its fields by ID.
    #[must_use]
    pub const fn info(&self) -> &'static TypeInfo {
        self.info
    }

    /// The bytes of the value: `info().layout().size()` of them, a valid value of the
    /// type, at no particular alignment. Read a field with `TypeInfo::field_bytes`.
    #[must_use]
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

/// The iterator of [`ViewModels::iter`]: every stored view-model, in ascending key order.
#[derive(Clone, Debug)]
pub struct ViewModelIter<'a> {
    values: btree_map::Values<'a, ComponentKey, Stored>,
}

impl<'a> Iterator for ViewModelIter<'a> {
    type Item = ViewModel<'a>;

    fn next(&mut self) -> Option<ViewModel<'a>> {
        self.values.next().map(|stored| ViewModel { info: stored.info, bytes: &stored.bytes })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.values.size_hint()
    }
}

impl ExactSizeIterator for ViewModelIter<'_> {}
