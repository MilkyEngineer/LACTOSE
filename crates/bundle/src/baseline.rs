//! The cooked baseline of a region.

use std::collections::BTreeMap;

use lactose_asset_api::{AssetId, AssetSet};
use lactose_core::{ComponentKey, NetId, Result};

use crate::ComponentData;
use crate::codec::{Reader, Writer, check_component_len, invalid, strictly_ascending};

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
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Column {
    component: ComponentKey,
    schema_version: u32,
    element_size: u32,
    bytes: Vec<u8>,
}

impl Column {
    /// The component this column holds.
    #[must_use]
    pub fn component(&self) -> ComponentKey {
        self.component
    }

    /// The schema version the values were cooked with. A loader whose registry disagrees
    /// must not bulk-copy the column.
    #[must_use]
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// The size of one value in bytes. Zero for a tag component.
    #[must_use]
    pub fn element_size(&self) -> u32 {
        self.element_size
    }

    /// The values: row `i` of the archetype occupies
    /// `i * element_size .. (i + 1) * element_size`.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// The rows of a baseline that share one exact component set.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Archetype {
    ids: Vec<NetId>,
    columns: Vec<Column>,
}

impl Archetype {
    /// The row IDs, strictly ascending. There is at least one.
    #[must_use]
    pub fn ids(&self) -> &[NetId] {
        &self.ids
    }

    /// One column per component, strictly ascending by [`ComponentKey`].
    #[must_use]
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// The column of `component`, if rows of this archetype have it.
    #[must_use]
    pub fn column(&self, component: ComponentKey) -> Option<&Column> {
        let index = self.columns.binary_search_by_key(&component, Column::component).ok()?;
        Some(&self.columns[index])
    }

    fn keys(&self) -> impl Iterator<Item = ComponentKey> + '_ {
        self.columns.iter().map(Column::component)
    }
}

/// The cooked baseline of one region: its entities as rows grouped by archetype, and the
/// region's asset set.
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
/// | component table | `u32` count, then per component `u64` key, `u32` schema version, `u32` element size, strictly ascending by key; exactly the components some archetype uses |
/// | archetypes | `u32` count, then each archetype, strictly ascending by component key list |
/// | archetype | `u32` component count, the keys as `u64` strictly ascending; `u32` row count, at least one; the row [`NetId`]s as `u64`, strictly ascending; then one column per key, in key order, of row count times element size bytes |
///
/// No [`NetId`] appears in two archetypes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Baseline {
    assets: AssetSet,
    archetypes: Vec<Archetype>,
}

impl Baseline {
    /// Builds a baseline from rows given in any order.
    ///
    /// Fails with `ErrorKind::InvalidData` if an ID is not a cooked ID or appears twice, if
    /// a row names a component twice, if a component's data is larger than
    /// [`MAX_COMPONENT_BYTES`](crate::MAX_COMPONENT_BYTES), or if a component's schema version or size differs between
    /// rows.
    pub fn from_rows(
        assets: AssetSet,
        rows: impl IntoIterator<Item = BaselineRow>,
    ) -> Result<Self> {
        let mut rows = keyed_rows(rows)?;
        rows.sort_by(|(a_keys, a), (b_keys, b)| a_keys.cmp(b_keys).then(a.id.cmp(&b.id)));

        let mut archetypes = Vec::new();
        for group in rows.chunk_by(|(a_keys, _), (b_keys, _)| a_keys == b_keys) {
            let (_, first) = &group[0];
            let mut columns: Vec<Column> = first
                .components
                .iter()
                .map(|data| Column {
                    component: data.component,
                    schema_version: data.schema_version,
                    element_size: data.bytes.len() as u32,
                    bytes: Vec::with_capacity(data.bytes.len() * group.len()),
                })
                .collect();
            for (_, row) in group {
                for (column, data) in columns.iter_mut().zip(&row.components) {
                    if data.bytes.len() != column.element_size as usize {
                        return Err(invalid("a component has two sizes in one baseline"));
                    }
                    if data.schema_version != column.schema_version {
                        return Err(invalid("a component has two schema versions in one baseline"));
                    }
                    column.bytes.extend_from_slice(&data.bytes);
                }
            }
            archetypes
                .push(Archetype { ids: group.iter().map(|(_, row)| row.id).collect(), columns });
        }

        let baseline = Self { assets, archetypes };
        baseline.validate()?;
        Ok(baseline)
    }

    /// The region's asset set: every asset the baseline's entities refer to, closed over
    /// declared dependencies by the cooker.
    #[must_use]
    pub fn assets(&self) -> &AssetSet {
        &self.assets
    }

    /// The archetypes, strictly ascending by component key list.
    #[must_use]
    pub fn archetypes(&self) -> &[Archetype] {
        &self.archetypes
    }

    /// The archetype holding the row `id`, if the baseline has that entity.
    #[must_use]
    pub fn archetype_of(&self, id: NetId) -> Option<&Archetype> {
        self.archetypes.iter().find(|archetype| archetype.ids.binary_search(&id).is_ok())
    }

    /// Whether the baseline has the entity `id`.
    #[must_use]
    pub fn contains(&self, id: NetId) -> bool {
        self.archetype_of(id).is_some()
    }

    /// The canonical encoding. Equal baselines encode to equal bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        self.seal().0
    }

    /// The baseline content hash (architecture section 15.2): the hash that ends the
    /// canonical encoding. Computing it encodes the document.
    #[must_use]
    pub fn content_hash(&self) -> u64 {
        self.seal().1
    }

    fn seal(&self) -> (Vec<u8>, u64) {
        let mut writer = Writer::new(MAGIC);
        writer.len(self.assets.len());
        for id in self.assets.iter() {
            writer.u64(id.to_bits());
        }
        let table = self.component_table().expect("a baseline is validated when it is built");
        writer.len(table.len());
        for (component, (schema_version, element_size)) in table {
            writer.u64(component.0);
            writer.u32(schema_version);
            writer.u32(element_size);
        }
        writer.len(self.archetypes.len());
        for archetype in &self.archetypes {
            writer.len(archetype.columns.len());
            for component in archetype.keys() {
                writer.u64(component.0);
            }
            writer.len(archetype.ids.len());
            for id in &archetype.ids {
                writer.u64(id.to_bits());
            }
            for column in &archetype.columns {
                writer.bytes(&column.bytes);
            }
        }
        writer.finish()
    }

    /// Decodes and validates untrusted bytes.
    ///
    /// Fails with `ErrorKind::VersionMismatch` for another format version and with
    /// `ErrorKind::InvalidData` for anything that is not the canonical encoding of a
    /// baseline. A successful decode re-encodes to exactly `bytes`.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut reader = Reader::open(bytes, MAGIC)?;

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

        let mut table = BTreeMap::new();
        for _ in 0..reader.count(16)? {
            let entry = (reader.component()?, (reader.u32()?, reader.u32()?));
            if table.last_key_value().is_some_and(|(&last, _)| entry.0 <= last) {
                return Err(invalid("component table is not strictly ascending"));
            }
            table.insert(entry.0, entry.1);
        }

        let mut archetypes = Vec::new();
        // An archetype is at least its two counts and one row ID.
        for _ in 0..reader.count(16)? {
            let mut columns = Vec::new();
            for _ in 0..reader.count(8)? {
                let component = reader.component()?;
                let Some(&(schema_version, element_size)) = table.get(&component) else {
                    return Err(invalid("archetype names a component outside the table"));
                };
                columns.push(Column { component, schema_version, element_size, bytes: Vec::new() });
            }
            let rows = reader.count(8)?;
            let ids = (0..rows).map(|_| reader.net_id()).collect::<Result<Vec<_>>>()?;
            for column in &mut columns {
                check_component_len(column.element_size as usize)?;
                let len = rows.checked_mul(column.element_size as usize);
                column.bytes = reader.take(len.ok_or(invalid("a column is too large"))?)?.to_vec();
            }
            archetypes.push(Archetype { ids, columns });
        }
        reader.finish()?;

        let baseline = Self { assets, archetypes };
        baseline.validate()?;
        if baseline.component_table()? != table {
            return Err(invalid("component table lists a component no archetype uses"));
        }
        Ok(baseline)
    }

    /// Schema version and element size per component, which must agree across archetypes.
    fn component_table(&self) -> Result<BTreeMap<ComponentKey, (u32, u32)>> {
        let mut table = BTreeMap::new();
        for column in self.archetypes.iter().flat_map(|archetype| &archetype.columns) {
            let entry = (column.schema_version, column.element_size);
            if *table.entry(column.component).or_insert(entry) != entry {
                return Err(invalid(
                    "a component has two schema versions or sizes in one baseline",
                ));
            }
        }
        Ok(table)
    }

    /// Checks every invariant the canonical form promises.
    fn validate(&self) -> Result<()> {
        for archetype in &self.archetypes {
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
            for column in &archetype.columns {
                check_component_len(column.element_size as usize)?;
                if column.bytes.len() != archetype.ids.len() * column.element_size as usize {
                    return Err(invalid("a column does not hold one value per row"));
                }
            }
        }
        for pair in self.archetypes.windows(2) {
            if !pair[0].keys().lt(pair[1].keys()) {
                return Err(invalid("archetypes are not strictly ascending"));
            }
        }
        self.component_table()?;
        let mut ids: Vec<_> = self.archetypes.iter().flat_map(|a| &a.ids).collect();
        ids.sort_unstable();
        strictly_ascending(ids, "an entity appears in two archetypes")
    }
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
            for data in &row.components {
                check_component_len(data.bytes.len())?;
            }
            Ok((keys, row))
        })
        .collect()
}
