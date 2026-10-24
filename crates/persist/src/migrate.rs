//! Schema migrations (architecture section 15.4): how a value written under an older
//! schema version of its type is brought to the registry's, field by field, in version
//! order.
//!
//! The binary keeps no historical layouts. A document carries the schema its values were
//! written with (version, size, field ID to offset and size) and a save carries the layout
//! digest beside it, so a value is decoded into its fields by ID under that schema, each
//! registered step rewrites the fields from one version to the next, and the result is
//! assembled under the registry's layout by field ID and validated.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::sync::Arc;

use bytemuck::{CheckedBitPattern, NoUninit};
use lactose_bundle::{ComponentData, ComponentSchema, Overlay, Record};
use lactose_core::{ComponentKey, Error, ErrorKind, FieldId, Result};
use lactose_reflect::{Persistence, TypeInfo, TypeRole};

use crate::codec::invalid;
use crate::diff::unknown;
use crate::schema::{LayoutDigest, SchemaTable, document_schema};

/// One migration step: a function over the fields of a value at version `from` that
/// leaves them at version `from + 1`.
///
/// A step is a plain function, not a closure, so a registered migration carries no state
/// and the set of migrations is a function of the build alone.
pub type MigrateFn = fn(&mut FieldValues) -> Result<()>;

/// A registered migration of one component or resource type from one schema version to
/// the next.
#[derive(Clone, Copy, Debug)]
pub struct Migration {
    /// The type the step migrates, by its key.
    pub component: ComponentKey,
    /// The schema version the step reads. It writes `from + 1`.
    pub from: u32,
    /// The step.
    pub run: MigrateFn,
}

/// The value a step edits: the fields of one component or resource value, keyed by field
/// ID, at the version the step reads.
///
/// A step reads fields it knows by [`get`](Self::get) or [`read`](Self::read), rewrites
/// them with [`set`](Self::set) or [`write`](Self::write), and drops them with
/// [`take`](Self::take); what it leaves is what the next step, or the assembly under the
/// registry's layout, sees. Bytes are the field's memory image, little-endian on every
/// supported target, canonical for a scalar. A field that holds a nested `#[value]` struct
/// is one field here, so a step over a nested layout change reads the old nested offsets
/// it hard-codes and writes the new value whole.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FieldValues {
    component: ComponentKey,
    version: u32,
    fields: BTreeMap<FieldId, Vec<u8>>,
}

impl FieldValues {
    /// The type whose value this is.
    #[must_use]
    pub fn component(&self) -> ComponentKey {
        self.component
    }

    /// The schema version the fields are at when the step runs.
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }

    /// The bytes of field `id`, if the value has it.
    #[must_use]
    pub fn get(&self, id: FieldId) -> Option<&[u8]> {
        self.fields.get(&id).map(Vec::as_slice)
    }

    /// Field `id` read as a `T`. Fails with `NotFound` if the value lacks the field and
    /// with `InvalidData` if its bytes are not a valid `T` of `T`'s size.
    pub fn read<T: CheckedBitPattern>(&self, id: FieldId) -> Result<T> {
        let bytes = self.get(id).ok_or_else(|| {
            Error::new(
                ErrorKind::NotFound,
                format!(
                    "{:#018x} has no field {} at version {}",
                    self.component.0, id.0, self.version
                ),
            )
        })?;
        if bytes.len() != size_of::<T>() {
            return Err(invalid(format!(
                "field {} of {:#018x} is {} bytes, read as {} bytes",
                id.0,
                self.component.0,
                bytes.len(),
                size_of::<T>()
            )));
        }
        bytemuck::checked::try_pod_read_unaligned(bytes).map_err(|_| {
            invalid(format!("field {} of {:#018x} is not a valid value", id.0, self.component.0))
        })
    }

    /// Sets field `id` to `bytes`, replacing any value it had.
    pub fn set(&mut self, id: FieldId, bytes: Vec<u8>) {
        self.fields.insert(id, bytes);
    }

    /// Sets field `id` to the memory image of `value`.
    pub fn write<T: NoUninit>(&mut self, id: FieldId, value: T) {
        self.set(id, bytemuck::bytes_of(&value).to_vec());
    }

    /// Removes field `id`, returning its bytes if the value had it.
    pub fn take(&mut self, id: FieldId) -> Option<Vec<u8>> {
        self.fields.remove(&id)
    }

    /// Whether the value has field `id`.
    #[must_use]
    pub fn contains(&self, id: FieldId) -> bool {
        self.fields.contains_key(&id)
    }

    /// The IDs of the fields the value has, ascending.
    #[must_use]
    pub fn ids(&self) -> FieldIds<'_> {
        FieldIds { keys: self.fields.keys() }
    }
}

/// The field IDs of a [`FieldValues`], ascending.
#[derive(Clone, Debug)]
pub struct FieldIds<'a> {
    keys: std::collections::btree_map::Keys<'a, FieldId, Vec<u8>>,
}

impl Iterator for FieldIds<'_> {
    type Item = FieldId;

    fn next(&mut self) -> Option<Self::Item> {
        self.keys.next().copied()
    }
}

/// Every registered migration step, keyed by (type, version read). Filled by each crate's
/// `register_migrations` function and handed to a [`Loader`](crate::Loader).
#[derive(Clone, Debug, Default)]
pub struct Migrations {
    steps: BTreeMap<(ComponentKey, u32), MigrateFn>,
}

impl Migrations {
    /// No migrations: every document must match the registry's schemas.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `migration`. Fails with `InvalidData` if a step for the same type and
    /// version is registered already: two crates cannot both own a type's migration.
    pub fn register(&mut self, migration: Migration) -> Result<()> {
        match self.steps.entry((migration.component, migration.from)) {
            Entry::Occupied(_) => Err(invalid(format!(
                "a step from version {} of {:#018x} is registered already",
                migration.from, migration.component.0
            ))),
            Entry::Vacant(slot) => {
                slot.insert(migration.run);
                Ok(())
            }
        }
    }

    /// The step that reads version `from` of `component`, if registered.
    #[must_use]
    pub fn step(&self, component: ComponentKey, from: u32) -> Option<MigrateFn> {
        self.steps.get(&(component, from)).copied()
    }

    /// The number of registered steps.
    #[must_use]
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Whether no step is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Brings `bytes`, one value of `info`'s type written under `schema` with the layout
    /// digest `layout`, to the registry's layout: the bytes of a valid value of the type
    /// as `info` describes it.
    ///
    /// The rule, in order:
    ///
    /// - `info.persistence()` is `Transient`: the all-zero value of `info`'s size, whatever
    ///   `schema`, `layout` and `bytes` say. A transient value is never saved (architecture
    ///   section 15.1): a save carries its presence, the loader zeroes it, and a layout
    ///   change to a type that stays transient breaks no save, needs no bump and no step,
    ///   and runs no check (a type made persistent since the save is read by the rules
    ///   below from the schema and digest it was saved under:
    ///   [`Loader::migrate`](crate::Loader::migrate)).
    /// - `schema.schema_version()` greater than `info.version()`: `VersionMismatch`, a
    ///   newer save than the build.
    /// - Equal versions: `layout` equal to [`LayoutDigest::of`]`(info)` is the fast path
    ///   (architecture section 15.4), `bytes` unchanged with no step and no validation
    ///   here; a different digest is `VersionMismatch`, a layout changed without a version
    ///   bump (a nested reorder, renumbered discriminants, a same-size type change: what
    ///   the document schema cannot see, which the reflection macros cannot see either,
    ///   so this is where it is caught, and the build-time guard D129 names is what keeps
    ///   it from being written).
    /// - A lower version: the steps for every version from `schema.schema_version()` up to
    ///   `info.version() - 1` must be registered (`VersionMismatch` naming the first
    ///   missing one); `bytes` is split into a [`FieldValues`] by `schema`'s fields, each
    ///   step runs in version order, and the result is assembled by the registry's layout:
    ///   every non-padding field of the type takes its bytes from the field of its ID,
    ///   which must be present and of the field's size (`VersionMismatch` otherwise,
    ///   naming the field), a field left over that the type lacks is `VersionMismatch`
    ///   naming it (a step must `take` what it retires), padding is zeroed and the whole
    ///   value is validated (`InvalidData`). The document's digest is not compared on this
    ///   path: the step is written against the old layout, and the assembly is by ID.
    ///
    /// A step's own error fails the call as returned.
    pub fn migrate(
        &self,
        info: &TypeInfo,
        schema: &ComponentSchema,
        layout: LayoutDigest,
        bytes: &[u8],
    ) -> Result<Vec<u8>> {
        let name = info.name();
        let size = info.layout().size();
        if info.persistence() == Persistence::Transient {
            return Ok(vec![0; size]);
        }
        if schema.component() != info.key() {
            return Err(invalid(format!(
                "a schema of {:#018x} is not one of `{name}`",
                schema.component().0
            )));
        }
        if bytes.len() != schema.element_size() as usize {
            return Err(invalid(format!(
                "a value of `{name}` is {} bytes and its schema states {}",
                bytes.len(),
                schema.element_size()
            )));
        }
        let (from, to) = (schema.schema_version(), info.version());
        if from > to {
            return Err(mismatch(format!(
                "`{name}` was saved at version {from}, newer than this build's {to}"
            )));
        }
        if from == to {
            if layout != LayoutDigest::of(info) || *schema != document_schema(info)? {
                return Err(mismatch(format!(
                    "`{name}` changed its layout at version {to} without a version bump"
                )));
            }
            return Ok(bytes.to_vec());
        }
        let mut steps = Vec::with_capacity((to - from) as usize);
        for version in from..to {
            let step = self.step(info.key(), version).ok_or_else(|| {
                mismatch(format!(
                    "no migration of `{name}` from version {version} (saved at {from}, this \
                     build at {to})"
                ))
            })?;
            steps.push((version, step));
        }
        let fields = schema.fields().iter().map(|field| {
            let at = field.offset as usize..(field.offset + field.size) as usize;
            (field.id, bytes[at].to_vec())
        });
        let mut value =
            FieldValues { component: info.key(), version: from, fields: fields.collect() };
        for (version, step) in steps {
            value.version = version;
            step(&mut value)?;
        }
        value.version = to;

        let mut out = vec![0; size];
        for field in info.layout().fields() {
            let Some(id) = field.id() else { continue };
            let bytes = value.take(id).ok_or_else(|| {
                mismatch(format!(
                    "migrating `{name}` to version {to} left field `{}` ({}) unset",
                    field.name(),
                    id.0
                ))
            })?;
            if bytes.len() != field.size() {
                return Err(mismatch(format!(
                    "migrating `{name}` to version {to} left field `{}` ({}) {} bytes, not {}",
                    field.name(),
                    id.0,
                    bytes.len(),
                    field.size()
                )));
            }
            out[field.offset()..field.offset() + field.size()].copy_from_slice(&bytes);
        }
        if let Some(id) = value.ids().next() {
            return Err(mismatch(format!(
                "migrating `{name}` to version {to} left field {}, which the type lacks",
                id.0
            )));
        }
        info.zero_padding(&mut out)?;
        info.validate(&out)?;
        Ok(out)
    }

    /// Brings every value `overlay` carries to the registry's schemas: each component of a
    /// `Spawn`, `SpawnFromBaseline` or `AddComponent` record through [`migrate`](Self::migrate)
    /// under the overlay's schema of that component and the digest `layouts` holds for it,
    /// and the overlay rebuilt against the same baseline hash with the table's schemas. An
    /// overlay whose every carried type is at the registry's version and digest, a
    /// transient type whose overlay schema equals the table's counting as such, is
    /// returned as the same `Arc`, equal by content and by digest, so a client's cached
    /// copy still matches; a migrated overlay is a new document with a new digest.
    /// `Destroy`, `RemoveComponent` and `OwnershipChange` carry no component bytes and pass
    /// through.
    ///
    /// A transient component is taken by presence, with no version or digest check and no
    /// step: its bytes are never read. When its overlay schema equals the table's they are
    /// kept as they are, zero or not, since `World::load_region` zeroes a transient
    /// component whatever a document says; when the schema differs (a layout change to a
    /// transient type) the record's component is rewritten as zeroes of the registry's size
    /// under the table's schema, since the loader requires every schema of a document to
    /// match the registry's. A component the registry now says is `derived` is dropped: it
    /// leaves its `Spawn` or `SpawnFromBaseline` record, which stays, and an `AddComponent`,
    /// `RemoveComponent` or `SetField` of it is dropped whole, since a derived value is
    /// rebuilt on load and never comes from a document, and the build declared it so. An
    /// overlay whose every record was dropped comes back empty, the one way this function
    /// yields an empty overlay besides an empty `overlay`, which comes back as the same
    /// `Arc`; [`Loader::migrate`](crate::Loader::migrate) makes it `None`.
    ///
    /// Fails as [`migrate`](Self::migrate) fails; with `NotFound` for a component the table
    /// does not know (a removed type: never dropped silently); with `InvalidData` for a
    /// carried component `layouts` lacks; and with `Unsupported` for a `SetField` record of
    /// a component whose version or digest differs, since one field cannot be migrated
    /// without the rest of its value (this crate's diff never writes a `SetField`, so none
    /// is in a save it wrote; one at the registry's version and digest passes through).
    pub fn migrate_overlay(
        &self,
        overlay: &Arc<Overlay>,
        layouts: &BTreeMap<ComponentKey, LayoutDigest>,
        schemas: &SchemaTable,
    ) -> Result<Arc<Overlay>> {
        let mut plans = BTreeMap::new();
        for schema in overlay.schemas() {
            let component = schema.component();
            let info = component_of(schemas, component)?;
            let plan = match info.persistence() {
                Persistence::Derived => Plan::Drop,
                Persistence::Transient if schemas.schema(component) == Some(schema) => Plan::Keep,
                Persistence::Transient => Plan::Zero,
                Persistence::Persistent => {
                    let layout = *layouts.get(&component).ok_or_else(|| {
                        invalid(format!("no layout digest for `{}`", info.name()))
                    })?;
                    let current = schemas.saved(component).expect("a persistent type is saved");
                    if *schema == current.schema && layout == current.layout {
                        Plan::Keep
                    } else {
                        Plan::Migrate(info, layout)
                    }
                }
            };
            plans.insert(component, plan);
        }
        // A `RemoveComponent` carries no bytes, so its component is not in the table above.
        for record in overlay.records() {
            if let Record::RemoveComponent { component, .. } = *record
                && let Entry::Vacant(slot) = plans.entry(component)
            {
                let info = component_of(schemas, component)?;
                let derived = info.persistence() == Persistence::Derived;
                slot.insert(if derived { Plan::Drop } else { Plan::Keep });
            }
        }
        if plans.values().all(|plan| matches!(plan, Plan::Keep)) {
            return Ok(Arc::clone(overlay));
        }

        let value = |data: &ComponentData| -> Result<Option<ComponentData>> {
            let bytes = match plans[&data.component] {
                Plan::Keep => data.bytes.clone(),
                Plan::Drop => return Ok(None),
                Plan::Zero => {
                    let size = schemas.saved(data.component).expect("saved").schema.element_size();
                    vec![0; size as usize]
                }
                Plan::Migrate(info, layout) => {
                    let schema = overlay.schema(data.component).expect("a carried component");
                    let mut bytes = self.migrate(info, schema, layout, &data.bytes)?;
                    schemas.saved(data.component).expect("saved").canonicalise(&mut bytes);
                    bytes
                }
            };
            Ok(Some(ComponentData { component: data.component, bytes }))
        };
        let mut records = Vec::with_capacity(overlay.records().len());
        for record in overlay.records() {
            let record = match record {
                Record::Spawn { id, components } => {
                    let components = components.iter().filter_map(|d| value(d).transpose());
                    Record::Spawn { id: *id, components: components.collect::<Result<_>>()? }
                }
                Record::SpawnFromBaseline { id, components } => {
                    let components = components.iter().filter_map(|d| value(d).transpose());
                    let components = components.collect::<Result<_>>()?;
                    Record::SpawnFromBaseline { id: *id, components }
                }
                Record::AddComponent { id, data } => match value(data)? {
                    Some(data) => Record::AddComponent { id: *id, data },
                    None => continue,
                },
                Record::RemoveComponent { component, .. } => match plans[component] {
                    Plan::Drop => continue,
                    Plan::Keep | Plan::Zero | Plan::Migrate(..) => record.clone(),
                },
                Record::SetField { component, .. } => {
                    match plans[component] {
                        Plan::Keep => record.clone(),
                        // A transient value loads zeroed, so a field of one says nothing.
                        Plan::Drop | Plan::Zero => continue,
                        Plan::Migrate(info, _) => {
                            return Err(Error::new(
                                ErrorKind::Unsupported,
                                format!(
                                    "a `SetField` of `{}` cannot be migrated without the rest \
                                     of its value",
                                    info.name()
                                ),
                            ));
                        }
                    }
                }
                Record::Destroy { .. } | Record::OwnershipChange { .. } => record.clone(),
            };
            records.push(record);
        }
        Ok(Arc::new(Overlay::new(overlay.baseline_hash(), schemas.schemas().cloned(), records)?))
    }
}

/// What [`Migrations::migrate_overlay`] does with each component an overlay names.
#[derive(Clone, Copy)]
enum Plan {
    /// At the registry's version and digest, or transient under the table's schema.
    Keep,
    /// Persistent at another version or digest: through [`Migrations::migrate`] under the
    /// digest the records carry.
    Migrate(&'static TypeInfo, LayoutDigest),
    /// Transient under another schema: zeroes of the registry's size.
    Zero,
    /// Derived now: never from a document.
    Drop,
}

/// The registry's description of `component`, which an overlay names: `NotFound` for a
/// type the table does not know, `InvalidData` for a resource.
fn component_of(schemas: &SchemaTable, component: ComponentKey) -> Result<&'static TypeInfo> {
    let info = schemas.info(component).ok_or_else(|| unknown(component))?;
    if info.role() != TypeRole::Component {
        return Err(invalid(format!("the resource `{}` is in an overlay", info.name())));
    }
    Ok(info)
}

fn mismatch(message: String) -> Error {
    Error::new(ErrorKind::VersionMismatch, message)
}
