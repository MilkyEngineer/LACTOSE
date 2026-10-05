//! The cooked baseline of a region.

use std::collections::BTreeSet;

use lactose_asset_api::{AssetId, AssetSet};
use lactose_core::{ComponentKey, NetId, Result};

use crate::ComponentData;
use crate::codec::{Reader, Writer, invalid, strictly_ascending};
use crate::schema::{self, ComponentSchema};

const MAGIC: [u8; 4] = *b"LBAS";

/// One cooked entity, as the cooker hands it to [`Baseline::from_rows`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BaselineRow {
    /// The entity's cooked ID, from [`cooked_net_id`](crate::cooked_net_id).
    pub id: NetId,
    /// The entity's components, in any order.
    pub components: Vec<ComponentData>,
}

/// One component's values for every row of an [`Archetype`], back to back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Column<'a> {
    /// The component this column holds. Its layout is [`Baseline::schema`] of this key.
    pub component: ComponentKey,
    /// The size of one value in bytes. Zero for a tag component.
    pub element_size: u32,
    /// The values: row `i` of the archetype occupies
    /// `i * element_size .. (i + 1) * element_size`.
    pub bytes: &'a [u8],
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct ColumnSpan {
    component: ComponentKey,
    element_size: u32,
    /// Where the column starts in the archetype's bytes.
    start: usize,
}

/// The rows of a baseline that share one exact component set.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Archetype {
    ids: Vec<NetId>,
    columns: Vec<ColumnSpan>,
    /// Every column, in column order, in one allocation.
    bytes: Vec<u8>,
}

impl Archetype {
    /// The row IDs, strictly ascending. There is at least one.
    #[must_use]
    pub fn ids(&self) -> &[NetId] {
        &self.ids
    }

    /// One column per component, strictly ascending by [`ComponentKey`].
    pub fn columns(&self) -> impl ExactSizeIterator<Item = Column<'_>> {
        self.columns.iter().map(|span| self.view(span))
    }

    /// The column of `component`, if rows of this archetype have it.
    #[must_use]
    pub fn column(&self, component: ComponentKey) -> Option<Column<'_>> {
        let index = self.columns.binary_search_by_key(&component, |span| span.component).ok()?;
        Some(self.view(&self.columns[index]))
    }

    /// Whether rows of this archetype have `component`.
    #[must_use]
    pub fn has(&self, component: ComponentKey) -> bool {
        self.columns.binary_search_by_key(&component, |span| span.component).is_ok()
    }

    fn view(&self, span: &ColumnSpan) -> Column<'_> {
        let len = self.ids.len() * span.element_size as usize;
        Column {
            component: span.component,
            element_size: span.element_size,
            bytes: &self.bytes[span.start..span.start + len],
        }
    }

    fn keys(&self) -> impl Iterator<Item = ComponentKey> + '_ {
        self.columns.iter().map(|span| span.component)
    }
}

/// The cooked baseline of one region: its entities as rows grouped by archetype, the
/// schema of every component they carry, and the region's asset set.
///
/// Component bytes are opaque here. A document is immutable and always canonical:
/// archetypes are ordered by component key list, rows by [`NetId`] and columns by
/// [`ComponentKey`], so the same content has exactly one in-memory form and one encoding.
///
/// # Encoding
///
/// After the shared header (see the crate documentation), in order:
///
/// | Part | Layout |
/// |---|---|
/// | asset set | `u32` count, then each [`AssetId`] as `u64`, strictly ascending, never zero |
/// | schema table | `u32` count, then each schema, strictly ascending by component key; exactly the components some archetype has |
/// | schema | `u64` component key, `u32` schema version, `u32` element size, `u32` field count, then per field `u16` field ID, `u32` offset, `u32` size, strictly ascending by field ID |
/// | archetypes | `u32` count, then each archetype, strictly ascending by component key list |
/// | archetype | `u32` component count, the keys as `u64` strictly ascending; `u32` row count, at least one; the row [`NetId`]s as `u64`, strictly ascending; then one column per key, in key order, of row count times element size bytes |
///
/// No [`NetId`] appears in two archetypes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Baseline {
    assets: AssetSet,
    schemas: Vec<ComponentSchema>,
    archetypes: Vec<Archetype>,
    /// Every row ID with the index of its archetype, ascending by ID.
    index: Vec<(NetId, u32)>,
    hash: u64,
}

impl Baseline {
    /// Builds a baseline from rows given in any order.
    ///
    /// `schemas` may hold more than the rows use, for instance a whole registry; the
    /// baseline keeps those of the components its rows have.
    ///
    /// Fails with `ErrorKind::InvalidData` if an ID is not a cooked ID or appears twice, if
    /// a row names a component twice, if a component has no schema or two, or if a
    /// component's bytes are not the size its schema states. Fails with
    /// `ErrorKind::ResourceExhausted` if the encoding would exceed
    /// [`MAX_DOCUMENT_BYTES`](crate::MAX_DOCUMENT_BYTES).
    pub fn from_rows(
        assets: AssetSet,
        schemas: impl IntoIterator<Item = ComponentSchema>,
        rows: impl IntoIterator<Item = BaselineRow>,
    ) -> Result<Self> {
        let mut rows = keyed_rows(rows)?;
        rows.sort_by(|(a_keys, a), (b_keys, b)| a_keys.cmp(b_keys).then(a.id.cmp(&b.id)));
        let used: BTreeSet<_> = rows.iter().flat_map(|(keys, _)| keys).copied().collect();
        let schemas = schema::select(schemas, &used)?;

        let mut archetypes = Vec::new();
        for group in rows.chunk_by(|(a_keys, _), (b_keys, _)| a_keys == b_keys) {
            let mut columns = Vec::new();
            let mut bytes = Vec::new();
            for (column, component) in group[0].0.iter().enumerate() {
                let schema = schema::find(&schemas, *component).expect("selected above");
                let element_size = schema.element_size();
                columns.push(ColumnSpan {
                    component: *component,
                    element_size,
                    start: bytes.len(),
                });
                for (_, row) in group {
                    let data = &row.components[column];
                    if data.bytes.len() != element_size as usize {
                        return Err(invalid("a component is not the size its schema states"));
                    }
                    bytes.extend_from_slice(&data.bytes);
                }
            }
            let ids = group.iter().map(|(_, row)| row.id).collect();
            archetypes.push(Archetype { ids, columns, bytes });
        }
        Self::assemble(assets, schemas, archetypes, None)
    }

    /// The region's asset set: every asset the baseline's entities refer to.
    #[must_use]
    pub fn assets(&self) -> &AssetSet {
        &self.assets
    }

    /// The schema of every component some row has, strictly ascending by component.
    #[must_use]
    pub fn schemas(&self) -> &[ComponentSchema] {
        &self.schemas
    }

    /// The schema the baseline's values of `component` were cooked with. A loader whose
    /// registry disagrees must not bulk-copy the column.
    #[must_use]
    pub fn schema(&self, component: ComponentKey) -> Option<&ComponentSchema> {
        schema::find(&self.schemas, component)
    }

    /// The archetypes, strictly ascending by component key list.
    #[must_use]
    pub fn archetypes(&self) -> &[Archetype] {
        &self.archetypes
    }

    /// The archetype holding the row `id`, if the baseline has that entity. A binary
    /// search over all rows.
    #[must_use]
    pub fn archetype_of(&self, id: NetId) -> Option<&Archetype> {
        let at = self.index.binary_search_by_key(&id, |&(row, _)| row).ok()?;
        Some(&self.archetypes[self.index[at].1 as usize])
    }

    /// Whether the baseline has the entity `id`.
    #[must_use]
    pub fn contains(&self, id: NetId) -> bool {
        self.archetype_of(id).is_some()
    }

    /// The baseline content hash (architecture section 15.2): the hash that ends the
    /// canonical encoding. It is kept from construction or decoding, so this is free.
    #[must_use]
    pub fn content_hash(&self) -> u64 {
        self.hash
    }

    /// The canonical encoding. Equal baselines encode to equal bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let sealed = seal(&self.assets, &self.schemas, &self.archetypes);
        sealed.expect("the size was checked when the baseline was built").0
    }

    /// Decodes and validates untrusted bytes.
    ///
    /// Fails with `ErrorKind::ResourceExhausted` for more than
    /// [`MAX_DOCUMENT_BYTES`](crate::MAX_DOCUMENT_BYTES), with `ErrorKind::VersionMismatch`
    /// for another format version, and with `ErrorKind::InvalidData` for anything that is
    /// not the canonical encoding of a baseline. A successful decode re-encodes to exactly
    /// `bytes`.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let (mut reader, hash) = Reader::open(bytes, MAGIC)?;

        let mut assets = AssetSet::new();
        let mut previous = AssetId::NONE;
        for _ in 0..reader.count(8)? {
            let id = AssetId::from_bits(reader.u64()?);
            if id <= previous {
                return Err(invalid("asset set is not strictly ascending and free of zero"));
            }
            assets.insert(id);
            previous = id;
        }

        let schemas = schema::read_table(&mut reader)?;
        // Lookups below are binary searches, so the order has to hold first.
        strictly_ascending(
            schemas.iter().map(ComponentSchema::component),
            "the schema table is not strictly ascending",
        )?;

        // An archetype is at least its two counts and one row ID.
        let count = reader.count(16)?;
        let mut archetypes = Vec::with_capacity(count);
        for _ in 0..count {
            let column_count = reader.count(8)?;
            let mut columns = Vec::with_capacity(column_count);
            let mut len = 0usize;
            let mut sizes = Vec::with_capacity(column_count);
            for _ in 0..column_count {
                let component = reader.component()?;
                let Some(schema) = schema::find(&schemas, component) else {
                    return Err(invalid("archetype has a component outside the schema table"));
                };
                sizes.push((component, schema.element_size()));
            }
            let rows = reader.count(8)?;
            let ids = (0..rows).map(|_| reader.net_id()).collect::<Result<Vec<_>>>()?;
            for (component, element_size) in sizes {
                columns.push(ColumnSpan { component, element_size, start: len });
                // Saturating, so an absurd product is simply more than `take` can find.
                len = len.saturating_add(rows.saturating_mul(element_size as usize));
            }
            let bytes = reader.take(len)?.to_vec();
            archetypes.push(Archetype { ids, columns, bytes });
        }
        reader.finish()?;
        Self::assemble(assets, schemas, archetypes, Some(hash))
    }

    /// Checks every invariant the canonical form promises and completes the document.
    /// `hash` is the trailing hash when the parts were decoded from sealed bytes.
    fn assemble(
        assets: AssetSet,
        schemas: Vec<ComponentSchema>,
        archetypes: Vec<Archetype>,
        hash: Option<u64>,
    ) -> Result<Self> {
        let mut used = BTreeSet::new();
        let mut index = Vec::with_capacity(archetypes.iter().map(|a| a.ids.len()).sum());
        for (number, archetype) in archetypes.iter().enumerate() {
            if archetype.ids.is_empty() {
                return Err(invalid("an archetype has no rows"));
            }
            if !archetype.ids.iter().all(|id| id.is_cooked()) {
                return Err(invalid("a baseline row is not a cooked entity"));
            }
            strictly_ascending(&archetype.ids, "archetype rows are not strictly ascending")?;
            strictly_ascending(
                archetype.keys(),
                "archetype components are not strictly ascending",
            )?;
            used.extend(archetype.keys());
            index.extend(archetype.ids.iter().map(|&id| (id, number as u32)));
        }
        for pair in archetypes.windows(2) {
            if !pair[0].keys().lt(pair[1].keys()) {
                return Err(invalid("archetypes are not strictly ascending"));
            }
        }
        schema::check_table(&schemas, &used)?;
        index.sort_unstable();
        strictly_ascending(index.iter().map(|&(id, _)| id), "an entity appears in two archetypes")?;

        let hash = match hash {
            Some(hash) => hash,
            None => seal(&assets, &schemas, &archetypes)?.1,
        };
        Ok(Self { assets, schemas, archetypes, index, hash })
    }
}

fn seal(
    assets: &AssetSet,
    schemas: &[ComponentSchema],
    archetypes: &[Archetype],
) -> Result<(Vec<u8>, u64)> {
    let mut writer = Writer::new(MAGIC);
    writer.count(assets.len());
    for id in assets.iter() {
        writer.u64(id.to_bits());
    }
    schema::write_table(&mut writer, schemas);
    writer.count(archetypes.len());
    for archetype in archetypes {
        writer.count(archetype.columns.len());
        for component in archetype.keys() {
            writer.u64(component.0);
        }
        writer.count(archetype.ids.len());
        for id in &archetype.ids {
            writer.u64(id.to_bits());
        }
        writer.bytes(&archetype.bytes);
    }
    writer.finish()
}

/// Checks each row on its own and pairs it with its sorted component key list.
fn keyed_rows(
    rows: impl IntoIterator<Item = BaselineRow>,
) -> Result<Vec<(Vec<ComponentKey>, BaselineRow)>> {
    rows.into_iter()
        .map(|mut row| {
            row.components.sort_by_key(|data| data.component);
            let keys: Vec<_> = row.components.iter().map(|data| data.component).collect();
            strictly_ascending(&keys, "a row names a component twice")?;
            Ok((keys, row))
        })
        .collect()
}
