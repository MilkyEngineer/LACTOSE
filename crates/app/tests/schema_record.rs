//! The record of every saved type's schema version, layout digest and field IDs, one file
//! per recorded registration set (the engine's today), and the guard over it (D129;
//! architecture section 15, "CI rejects ID reuse"); `README.md`, "The saved-type record",
//! is the contract.
//!
//! `the_engine_record_is_current` derives the engine set's record from the schemas the
//! loader compares and the registry's field lists, and fails on a type whose layout
//! changed at an unchanged version, on a field ID reused, and on any other
//! difference from `tests/schema_record/engine.txt` until the record is rewritten. The
//! other tests hold the guard's rules over small registries of their own, each type
//! declared twice under one key: before a change and after it.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use lactose_app::Game;
use lactose_core::{ComponentKey, Error, ErrorKind, PlainArray, Result};
use lactose_persist::SchemaTable;
use lactose_reflect::{FieldKind, Persistence, Reflect, Registry, StructInfo, TypeRole};

/// The variable that selects how [`guard`] treats a record that differs from the build.
const MODE_VARIABLE: &str = "LACTOSE_SCHEMA_RECORD";

/// The command that rewrites every record after a deliberate change, which a failure names.
const UPDATE: &str = "LACTOSE_SCHEMA_RECORD=update cargo nextest run -p lactose-app --test integration schema_record::";

/// The same, also taking a field's new name under its old ID as a rename.
const RENAME: &str = "LACTOSE_SCHEMA_RECORD=rename cargo nextest run -p lactose-app --test integration schema_record::";

/// The engine's registration set: the game with no crate, whose `Game::types()` is
/// `TypeTable::new()` and the engine crates' `register_types`, which every game's set
/// contains.
static ENGINE: Game = Game { name: "engine", crates: &[], order: None, sessions: None };

#[test]
fn the_engine_record_is_current() {
    let types = ENGINE.types().expect("the engine's types register");
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/schema_record/engine.txt");
    if let Err(message) = guard("engine", types.registry(), &path, Mode::from_env()) {
        panic!("{message}");
    }
}

// ---- The record ----

/// One registration set's record: every type `SchemaTable::schemas()` holds, by key.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
struct Record {
    types: BTreeMap<ComponentKey, SavedType>,
}

/// One saved type: the key, version and digest the loader compares
/// (`SchemaTable::schemas()`), and the registry's name, role, policy and field IDs.
#[derive(Clone, PartialEq, Eq, Debug)]
struct SavedType {
    name: String,
    role: TypeRole,
    persistence: Persistence,
    version: u32,
    digest: u64,
    fields: Scope,
}

/// The field IDs of one struct: a saved type's own, or those of the `#[value]` struct one
/// of its fields holds, which are that struct's own.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
struct Scope {
    fields: BTreeMap<u16, FieldEntry>,
    retired: BTreeSet<u16>,
}

/// What an ID names: the field's Rust name, its type as text, and the IDs inside it when it
/// holds a `#[value]` struct or an array of them (empty otherwise).
#[derive(Clone, PartialEq, Eq, Debug)]
struct FieldEntry {
    name: String,
    type_name: String,
    nested: Scope,
}

impl Record {
    /// The record of `registry`, as `lactose-app` builds a save's table from it.
    fn of(registry: &Registry) -> Result<Self> {
        let table = SchemaTable::from_registry(registry)?;
        let mut types = BTreeMap::new();
        for schema in table.schemas() {
            let info = table.info(schema.component()).expect("a saved type is registered");
            let saved = SavedType {
                name: info.name().to_owned(),
                role: info.role(),
                persistence: info.persistence(),
                version: schema.schema_version(),
                digest: schema.layout_digest(),
                fields: Scope::of(info.layout()),
            };
            types.insert(schema.component(), saved);
        }
        Ok(Self { types })
    }

    /// The record as its file holds it: a header naming `set` and the command, then one
    /// block per type ascending by key, a line of the type and one line per field ID and
    /// per retired ID, a nested struct's IDs under the outer ID (`1.2`), ascending.
    fn render(&self, set: &str) -> String {
        let mut out = format!(
            "# The saved types of the `{set}` registration set, as `tests/schema_record.rs`\n\
             # derives them; `crates/app/README.md`, \"The saved-type record\", says what each\n\
             # line holds and when the record changes. It is compared with LF line endings and\n\
             # rewritten only by this command, or by it with `rename` in place of `update`:\n\
             #   {UPDATE}\n"
        );
        for (key, saved) in &self.types {
            let _ = writeln!(
                out,
                "{} key={:#018x} {} {} version={} digest={:#018x}",
                saved.name,
                key.0,
                role_name(saved.role),
                persistence_name(saved.persistence),
                saved.version,
                saved.digest
            );
            saved.fields.render("", &mut out);
        }
        out
    }

    /// Reads a record file back. A line it cannot read, a field under an ID the record does
    /// not hold or under one that holds no struct, an ID stated twice, an ID both a field
    /// and retired in one scope and a key that is not its name's are `InvalidData` naming
    /// the line.
    fn parse(text: &str) -> Result<Self> {
        let mut types = BTreeMap::new();
        let mut current: Option<(ComponentKey, SavedType)> = None;
        for (index, line) in text.lines().enumerate() {
            let bad = |what: &str| {
                Error::new(ErrorKind::InvalidData, format!("line {}: {what}: `{line}`", index + 1))
            };
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some(entry) = line.strip_prefix("  ") else {
                if let Some((key, saved)) = current.take() {
                    types.insert(key, saved);
                }
                let (key, saved) = parse_type(line).ok_or_else(|| bad("not a type line"))?;
                if types.contains_key(&key) {
                    return Err(bad("a type stated twice"));
                }
                current = Some((key, saved));
                continue;
            };
            let (_, saved) = current.as_mut().ok_or_else(|| bad("a field before any type"))?;
            if let Some(path) = entry.strip_prefix("retired ") {
                let (parent, id) = parse_path(path).ok_or_else(|| bad("not an ID path"))?;
                let scope = saved.fields.walk(&parent).map_err(bad)?;
                if scope.fields.contains_key(&id) {
                    return Err(bad("an ID both a field and retired"));
                }
                if !scope.retired.insert(id) {
                    return Err(bad("an ID retired twice"));
                }
            } else {
                let (path, rest) = entry.split_once(' ').ok_or_else(|| bad("not a field line"))?;
                let (name, type_name) =
                    rest.split_once(": ").ok_or_else(|| bad("not a field line"))?;
                let (parent, id) = parse_path(path).ok_or_else(|| bad("not an ID path"))?;
                let scope = saved.fields.walk(&parent).map_err(bad)?;
                if scope.retired.contains(&id) {
                    return Err(bad("an ID both a field and retired"));
                }
                let field = FieldEntry {
                    name: name.to_owned(),
                    type_name: type_name.to_owned(),
                    nested: Scope::default(),
                };
                if scope.fields.insert(id, field).is_some() {
                    return Err(bad("an ID stated twice"));
                }
            }
        }
        if let Some((key, saved)) = current {
            types.insert(key, saved);
        }
        Ok(Self { types })
    }

    /// What breaks the record in `live`, one message per break, each naming its type. A
    /// type the record lacks, one `live` lacks and a version that rose (with any digest)
    /// break nothing; they are changes the record takes. `renames` takes a field's new name
    /// under its recorded ID, its type unchanged, as a rename rather than a reuse.
    fn breaks(&self, live: &Self, renames: bool) -> Vec<String> {
        let mut out = Vec::new();
        for (key, was) in &self.types {
            let Some(now) = live.types.get(key) else { continue };
            let name = &now.name;
            // Transient types too: no load reads a transient type's version or digest, but
            // one made persistent later is read from what it was saved under
            // (`crates/persist/README.md`, "Migrations"), and the record is the only place
            // that says which layout an older save holds.
            if now.version < was.version {
                out.push(format!(
                    "`{name}` is at version {}, below the record's {}: a version never falls, \
                     and a save written at the higher one is refused as newer than the build",
                    now.version, was.version
                ));
            } else if now.version == was.version && now.digest != was.digest {
                // No step runs for a transient type, so its remedy names none.
                let remedy = if now.persistence == Persistence::Transient {
                    format!(
                        "Raise its `version`, then run `{UPDATE}`. No load needs the bump while \
                         the type is transient, but one made persistent later is read from the \
                         version and digest it was saved under, and the commit that makes it \
                         persistent registers the steps from each version it was saved at"
                    )
                } else {
                    let transient = if was.persistence == Persistence::Transient {
                        ". A transient type takes the bump too: one made persistent later is \
                         read from the version and digest it was saved under"
                    } else {
                        ""
                    };
                    format!(
                        "Raise its `version` and register the step from {} \
                         (`crates/persist/README.md`, \"The registration a game crate \
                         writes\"), then run `{UPDATE}`{transient}",
                        now.version
                    )
                };
                out.push(format!(
                    "`{name}` changed its layout at version {}: the record holds digest \
                     {:#018x}, the build {:#018x}. {remedy}",
                    now.version, was.digest, now.digest
                ));
            }
            // An equal digest means equal IDs, offsets and sizes at every depth, so the
            // nested IDs are compared even where a struct's name changed.
            let same = now.digest == was.digest;
            was.fields.breaks(&now.fields, name, "", (renames, same), &mut out);
        }
        out
    }

    /// One line per type `live` adds, drops or changes against this record.
    fn changes(&self, live: &Self) -> Vec<String> {
        let keys: BTreeSet<_> = self.types.keys().chain(live.types.keys()).collect();
        keys.into_iter()
            .filter_map(|key| match (self.types.get(key), live.types.get(key)) {
                (None, Some(now)) => Some(format!("`{}` is new", now.name)),
                (Some(was), None) => Some(format!("`{}` is no longer saved", was.name)),
                (Some(was), Some(now)) if was != now => Some(format!("`{}` changed", now.name)),
                _ => None,
            })
            .collect()
    }
}

impl Scope {
    /// The IDs of `layout`, nested structs included; padding has none.
    fn of(layout: &StructInfo) -> Self {
        let fields = layout
            .fields()
            .iter()
            .filter_map(|field| {
                let (id, kind) = (field.id()?, field.kind());
                let entry = FieldEntry {
                    name: field.name().to_owned(),
                    type_name: type_text(&kind, field.type_name()),
                    nested: nested(&kind).map_or_else(Self::default, Self::of),
                };
                Some((id.0, entry))
            })
            .collect();
        Self { fields, retired: layout.retired().iter().map(|id| id.0).collect() }
    }

    fn render(&self, prefix: &str, out: &mut String) {
        for (id, field) in &self.fields {
            let _ = writeln!(out, "  {prefix}{id} {}: {}", field.name, field.type_name);
            field.nested.render(&format!("{prefix}{id}."), out);
        }
        for id in &self.retired {
            let _ = writeln!(out, "  retired {prefix}{id}");
        }
    }

    /// The scope inside the field at `path`, this one for an empty path; otherwise what is
    /// wrong with the path.
    fn walk(&mut self, path: &[u16]) -> Result<&mut Self, &'static str> {
        path.iter().try_fold(self, |scope, id| {
            let field = scope.fields.get_mut(id).ok_or("no such field")?;
            if !holds_struct(&field.type_name) {
                return Err("an ID under a field that holds no struct");
            }
            Ok(&mut field.nested)
        })
    }

    /// The reuses of this scope's IDs in `now`, the same struct in the build: an ID that
    /// names another field, one that left the fields without being retired, and a retired
    /// one given back. `renames` takes a new name under an ID whose type is unchanged as a
    /// rename. A nested struct is compared while the field holding it holds a struct of the
    /// same name, or while `same`, the type's digest unchanged, says the layout is equal at
    /// every depth; otherwise another struct there has IDs of its own.
    fn breaks(
        &self,
        now: &Self,
        name: &str,
        prefix: &str,
        (renames, same): (bool, bool),
        out: &mut Vec<String>,
    ) {
        let retire = || match prefix.strip_suffix('.') {
            None => format!("`{name}`'s `retired`"),
            Some(outer) => format!("the `retired` of the `#[value]` struct field {outer} holds"),
        };
        for (id, was) in &self.fields {
            match now.fields.get(id) {
                Some(field) => {
                    let retyped = field.type_name != was.type_name;
                    if field.name != was.name && (!renames || retyped) {
                        let rename = if retyped {
                            format!(
                                "a rename that also changes the type is two commits, the \
                                 rename first under `{RENAME}`"
                            )
                        } else {
                            format!("if it is `{}` renamed, run `{RENAME}`", was.name)
                        };
                        out.push(format!(
                            "field ID {prefix}{id} of `{name}` names `{}` in the record and \
                             `{}` in the build: an ID names one field for ever. If `{}` is a \
                             new field, give it an ID the struct has never used and add {id} \
                             to {}; {rename}",
                            was.name,
                            field.name,
                            field.name,
                            retire(),
                        ));
                    }
                    if same || innermost(&field.type_name) == innermost(&was.type_name) {
                        let inner = format!("{prefix}{id}.");
                        was.nested.breaks(&field.nested, name, &inner, (renames, same), out);
                    }
                }
                None if now.retired.contains(id) => {}
                None => out.push(format!(
                    "field ID {prefix}{id} of `{name}` (`{}`) left the fields without being \
                     retired, so a later field could take it: add {id} to {}",
                    was.name,
                    retire()
                )),
            }
        }
        for id in self.retired.difference(&now.retired) {
            out.push(format!(
                "field ID {prefix}{id} of `{name}` is retired in the record and not in the \
                 build: a retired ID is never given back; put it back in {}",
                retire()
            ));
        }
    }
}

/// The struct whose IDs a field of `kind` holds: its own, or an array's element's.
fn nested(kind: &FieldKind) -> Option<&'static StructInfo> {
    match kind {
        FieldKind::Struct(info) => Some(info),
        FieldKind::Array { element, .. } => nested(element),
        _ => None,
    }
}

/// A field's type as the record writes it: the type tag, a `#[value]` struct's as
/// `struct Name` so the text says which fields hold IDs of their own, and an array as
/// `[element; len]`.
fn type_text(kind: &FieldKind, tag: &str) -> String {
    match kind {
        FieldKind::Array { element, element_name, len } => {
            format!("[{}; {len}]", type_text(element, element_name))
        }
        FieldKind::Struct(_) => format!("struct {tag}"),
        _ => tag.to_owned(),
    }
}

/// The element type of a type text, through every array.
fn innermost(type_name: &str) -> &str {
    let inner = type_name.trim_start_matches('[');
    inner.split(';').next().unwrap_or(inner)
}

/// Whether a field of this type text holds a `#[value]` struct, itself or as an array's
/// element, and so IDs of its own.
fn holds_struct(type_name: &str) -> bool {
    innermost(type_name).starts_with("struct ")
}

fn role_name(role: TypeRole) -> &'static str {
    match role {
        TypeRole::Component => "component",
        TypeRole::Resource => "resource",
    }
}

fn persistence_name(persistence: Persistence) -> &'static str {
    match persistence {
        Persistence::Persistent => "persistent",
        Persistence::Transient => "transient",
        Persistence::Derived => "derived",
    }
}

/// A type line: `name key=0x.. role persistence version=N digest=0x..`, the key the name's.
fn parse_type(line: &str) -> Option<(ComponentKey, SavedType)> {
    let mut tokens = line.split(' ');
    let name = tokens.next()?;
    let key = parse_hex(tokens.next()?.strip_prefix("key=")?)?;
    let role = match tokens.next()? {
        "component" => TypeRole::Component,
        "resource" => TypeRole::Resource,
        _ => return None,
    };
    let persistence = match tokens.next()? {
        "persistent" => Persistence::Persistent,
        "transient" => Persistence::Transient,
        _ => return None,
    };
    let version = tokens.next()?.strip_prefix("version=")?.parse().ok()?;
    let digest = parse_hex(tokens.next()?.strip_prefix("digest=")?)?;
    let key = ComponentKey(key);
    if tokens.next().is_some() || key != ComponentKey::from_name(name) {
        return None;
    }
    let fields = Scope::default();
    Some((key, SavedType { name: name.to_owned(), role, persistence, version, digest, fields }))
}

fn parse_hex(text: &str) -> Option<u64> {
    u64::from_str_radix(text.strip_prefix("0x")?, 16).ok()
}

/// `1.2.3` as the path of the outer fields, `[1, 2]`, and the ID, `3`.
fn parse_path(text: &str) -> Option<(Vec<u16>, u16)> {
    let mut ids = text.split('.').map(|id| id.parse().ok()).collect::<Option<Vec<u16>>>()?;
    let id = ids.pop()?;
    Some((ids, id))
}

// ---- The guard ----

/// How [`guard`] treats a record that differs from the build.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    /// Fail on any difference: CI, and every run without the variable.
    Check,
    /// Rewrite the record unless something breaks it; `renames` takes a field's new name
    /// under its recorded ID, of an unchanged type, as a rename.
    Update { renames: bool },
}

impl Mode {
    /// `Check`, unless `LACTOSE_SCHEMA_RECORD` is `update` or `rename`.
    fn from_env() -> Self {
        match std::env::var(MODE_VARIABLE).as_deref() {
            Err(std::env::VarError::NotPresent) => Self::Check,
            Ok("update") => Self::Update { renames: false },
            Ok("rename") => Self::Update { renames: true },
            other => panic!("{MODE_VARIABLE} is `update`, `rename` or unset, not {other:?}"),
        }
    }
}

/// Compares the record of `registry` with the file at `path`, the record of the set named
/// `set`. Every mode fails, rewriting nothing, when the build breaks the record; `Check`
/// fails on any other difference too, and `Update` rewrites the file then, unless what it
/// would write does not read back as the build's record. A missing file is an empty record,
/// written whole by `Update`. The file is compared with LF line endings, so a checkout that
/// gives it CRLF compares equal.
fn guard(set: &str, registry: &Registry, path: &Path, mode: Mode) -> Result<(), String> {
    let shown = path.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap_or(path).display();
    let live = Record::of(registry).map_err(|error| format!("the `{set}` set: {error}"))?;
    let text = match std::fs::read_to_string(path) {
        Ok(text) => Some(text.replace("\r\n", "\n")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("{shown}: {error}")),
    };
    let recorded = match &text {
        Some(text) => Record::parse(text)
            .map_err(|error| format!("{shown}: {error}; resolve it, then run `{UPDATE}`"))?,
        None => Record::default(),
    };
    let renames = matches!(mode, Mode::Update { renames: true });
    let breaks = recorded.breaks(&live, renames);
    if !breaks.is_empty() {
        let rewritten = if mode == Mode::Check { "" } else { " It is not rewritten." };
        return Err(format!(
            "the `{set}` registration set breaks its record, {shown}:{rewritten}\n- {}",
            breaks.join("\n- ")
        ));
    }
    let rendered = live.render(set);
    if text.as_deref() == Some(rendered.as_str()) {
        return Ok(());
    }
    match mode {
        Mode::Check => {
            let mut changes = recorded.changes(&live);
            if text.is_none() {
                changes = vec!["there is no record".to_owned()];
            } else if changes.is_empty() {
                changes.push("its text is not the build's rendering of it".to_owned());
            }
            Err(format!(
                "the record of the `{set}` registration set, {shown}, is out of date: {}. A \
                 deliberate change rewrites it in the commit that makes the change: run `{UPDATE}`",
                changes.join("; ")
            ))
        }
        Mode::Update { .. } => {
            match Record::parse(&rendered) {
                Ok(read) if read == live => {}
                result => {
                    let why = result.err().map_or_else(
                        || "it reads back as another record".to_owned(),
                        |error| error.to_string(),
                    );
                    return Err(format!(
                        "the record of the `{set}` registration set, {shown}, would not read \
                         back as the build's ({why}): a key or type tag the record's lines \
                         cannot hold. It is not rewritten."
                    ));
                }
            }
            std::fs::write(path, rendered).map_err(|error| format!("{shown}: {error}"))
        }
    }
}

// ---- The guard's rules over small registries ----

/// Two `u32` fields, at version 1.
mod pair {
    #[lactose_reflect::component(key = "record_test::pair", persistent)]
    pub struct Pair {
        #[field(id = 1)]
        pub a: u32,
        #[field(id = 2)]
        pub b: u32,
    }
}

/// The two fields swapped, a same-size layout change, at version 1.
mod pair_swapped {
    #[lactose_reflect::component(key = "record_test::pair", persistent)]
    pub struct Pair {
        #[field(id = 2)]
        pub b: u32,
        #[field(id = 1)]
        pub a: u32,
    }
}

/// The two fields swapped, at version 2.
mod pair_swapped_bumped {
    #[lactose_reflect::component(key = "record_test::pair", persistent, version = 2)]
    pub struct Pair {
        #[field(id = 2)]
        pub b: u32,
        #[field(id = 1)]
        pub a: u32,
    }
}

/// `b` deleted and its ID given to a new field of its type: the same layout and digest.
mod pair_reused {
    #[lactose_reflect::component(key = "record_test::pair", persistent)]
    pub struct Pair {
        #[field(id = 1)]
        pub a: u32,
        #[field(id = 2)]
        pub c: u32,
    }
}

/// The same reuse under a version bump.
mod pair_reused_bumped {
    #[lactose_reflect::component(key = "record_test::pair", persistent, version = 2)]
    pub struct Pair {
        #[field(id = 1)]
        pub a: u32,
        #[field(id = 2)]
        pub c: u32,
    }
}

/// `b` deleted and a new field under a new ID, at version 2, with `b`'s ID not retired.
mod pair_dropped {
    #[lactose_reflect::component(key = "record_test::pair", persistent, version = 2)]
    pub struct Pair {
        #[field(id = 1)]
        pub a: u32,
        #[field(id = 3)]
        pub c: u32,
    }
}

/// The same with `b`'s ID retired.
mod pair_retired {
    #[lactose_reflect::component(key = "record_test::pair", persistent, version = 2, retired = [2])]
    pub struct Pair {
        #[field(id = 1)]
        pub a: u32,
        #[field(id = 3)]
        pub c: u32,
    }
}

/// `b`'s retired ID given back to a field, at version 3.
mod pair_given_back {
    #[lactose_reflect::component(key = "record_test::pair", persistent, version = 3)]
    pub struct Pair {
        #[field(id = 1)]
        pub a: u32,
        #[field(id = 3)]
        pub c: u32,
        #[field(id = 2)]
        pub d: u32,
    }
}

/// `a` renamed: its ID, type and offset unchanged.
mod pair_renamed {
    #[lactose_reflect::component(key = "record_test::pair", persistent)]
    pub struct Pair {
        #[field(id = 1)]
        pub first: u32,
        #[field(id = 2)]
        pub b: u32,
    }
}

/// `b` deleted and its ID given to a new field of another type and size, at version 2.
mod pair_retyped {
    #[lactose_reflect::component(key = "record_test::pair", persistent, version = 2)]
    pub struct Pair {
        #[field(id = 2)]
        pub c: u64,
        #[field(id = 1)]
        pub a: u32,
        #[field(padding)]
        _pad: lactose_reflect::Padding<4>,
    }
}

/// A key with a space, which a record's type line cannot hold.
#[lactose_reflect::component(key = "record_test::two words", persistent)]
pub struct Spaced(#[field(id = 1)] pub u32);

/// A transient type, then its fields swapped at one version, then persistent so, and the
/// same two changes each under a bump.
mod marker {
    #[lactose_reflect::component(key = "record_test::marker", transient)]
    pub struct Marker {
        #[field(id = 1)]
        pub a: u32,
        #[field(id = 2)]
        pub b: u32,
    }
}

mod marker_swapped {
    #[lactose_reflect::component(key = "record_test::marker", transient)]
    pub struct Marker {
        #[field(id = 2)]
        pub b: u32,
        #[field(id = 1)]
        pub a: u32,
    }
}

mod marker_persistent {
    #[lactose_reflect::component(key = "record_test::marker", persistent)]
    pub struct Marker {
        #[field(id = 2)]
        pub b: u32,
        #[field(id = 1)]
        pub a: u32,
    }
}

mod marker_swapped_bumped {
    #[lactose_reflect::component(key = "record_test::marker", transient, version = 2)]
    pub struct Marker {
        #[field(id = 2)]
        pub b: u32,
        #[field(id = 1)]
        pub a: u32,
    }
}

mod marker_persistent_bumped {
    #[lactose_reflect::component(key = "record_test::marker", persistent, version = 2)]
    pub struct Marker {
        #[field(id = 2)]
        pub b: u32,
        #[field(id = 1)]
        pub a: u32,
    }
}

/// A derived component, which no record holds.
#[lactose_reflect::component(key = "record_test::handle", derived)]
pub struct Handle(#[field(id = 1)] pub u64);

/// A component holding a `#[value]` struct and an array of them, then the struct's `y`
/// deleted and its ID given to `z`, at version 2.
mod outer {
    use super::PlainArray;

    #[lactose_reflect::value]
    pub struct Inner {
        #[field(id = 1)]
        pub x: u32,
        #[field(id = 2)]
        pub y: u32,
    }

    #[lactose_reflect::component(key = "record_test::outer", persistent)]
    pub struct Outer {
        #[field(id = 1)]
        pub one: Inner,
        #[field(id = 2)]
        pub many: PlainArray<Inner, 2>,
    }
}

mod outer_reused {
    use super::PlainArray;

    #[lactose_reflect::value]
    pub struct Inner {
        #[field(id = 1)]
        pub x: u32,
        #[field(id = 2)]
        pub z: u32,
    }

    #[lactose_reflect::component(key = "record_test::outer", persistent, version = 2)]
    pub struct Outer {
        #[field(id = 1)]
        pub one: Inner,
        #[field(id = 2)]
        pub many: PlainArray<Inner, 2>,
    }
}

/// A component holding a `#[value]` struct, then that struct renamed: alone, with its two
/// fields' names swapped, and with `y` deleted and its ID given to `z`, each at version 1
/// and so at one digest, since the digest holds neither a struct's name nor field names.
mod nest {
    #[lactose_reflect::value]
    pub struct Inner {
        #[field(id = 1)]
        pub x: u32,
        #[field(id = 2)]
        pub y: u32,
    }

    #[lactose_reflect::component(key = "record_test::nest", persistent)]
    pub struct Nest {
        #[field(id = 1)]
        pub one: Inner,
    }
}

mod nest_renamed {
    #[lactose_reflect::value]
    pub struct Renamed {
        #[field(id = 1)]
        pub x: u32,
        #[field(id = 2)]
        pub y: u32,
    }

    #[lactose_reflect::component(key = "record_test::nest", persistent)]
    pub struct Nest {
        #[field(id = 1)]
        pub one: Renamed,
    }
}

mod nest_renamed_swapped {
    #[lactose_reflect::value]
    pub struct Renamed {
        #[field(id = 1)]
        pub y: u32,
        #[field(id = 2)]
        pub x: u32,
    }

    #[lactose_reflect::component(key = "record_test::nest", persistent)]
    pub struct Nest {
        #[field(id = 1)]
        pub one: Renamed,
    }
}

mod nest_renamed_reused {
    #[lactose_reflect::value]
    pub struct Renamed {
        #[field(id = 1)]
        pub x: u32,
        #[field(id = 2)]
        pub z: u32,
    }

    #[lactose_reflect::component(key = "record_test::nest", persistent)]
    pub struct Nest {
        #[field(id = 1)]
        pub one: Renamed,
    }
}

/// The record of a registry of `T` and `U`.
fn record<T: Reflect, U: Reflect>() -> Record {
    let mut registry = Registry::new();
    registry.register::<T>().unwrap();
    registry.register::<U>().unwrap();
    Record::of(&registry).unwrap()
}

/// What breaks the record of `T` beside `Handle` in that of `U` beside it.
fn breaks_between<T: Reflect, U: Reflect>(renames: bool) -> Vec<String> {
    record::<T, Handle>().breaks(&record::<U, Handle>(), renames)
}

#[test]
fn a_same_size_layout_change_without_a_bump_breaks_the_record() {
    let before = record::<pair::Pair, Handle>().types[&pair::Pair::KEY].digest;
    let after = record::<pair_swapped::Pair, Handle>().types[&pair::Pair::KEY].digest;
    assert_ne!(before, after);
    let breaks = breaks_between::<pair::Pair, pair_swapped::Pair>(false);
    assert_eq!(breaks.len(), 1, "{breaks:#?}");
    assert!(breaks[0].starts_with("`record_test::pair` changed its layout at version 1"));
    assert!(!breaks[0].contains("transient"));
    assert!(
        breaks[0].contains(&format!("{before:#018x}"))
            && breaks[0].contains(&format!("{after:#018x}"))
    );
    assert!(breaks[0].contains(UPDATE));
}

#[test]
fn a_version_that_rose_takes_any_digest() {
    assert!(breaks_between::<pair::Pair, pair_swapped_bumped::Pair>(false).is_empty());
    let breaks = breaks_between::<pair_swapped_bumped::Pair, pair_swapped::Pair>(false);
    assert_eq!(breaks.len(), 1, "{breaks:#?}");
    assert!(breaks[0].starts_with("`record_test::pair` is at version 1, below the record's 2"));
}

#[test]
fn a_transient_type_is_held_to_its_digest_like_a_persistent_one() {
    // Transient in the build: the bump alone, since no step runs for a transient type.
    for breaks in [
        breaks_between::<marker::Marker, marker_swapped::Marker>(false),
        breaks_between::<marker_persistent::Marker, marker::Marker>(false),
    ] {
        assert_eq!(breaks.len(), 1, "{breaks:#?}");
        assert!(breaks[0].starts_with("`record_test::marker` changed its layout at version 1"));
        assert!(breaks[0].contains(&format!("Raise its `version`, then run `{UPDATE}`")));
        assert!(breaks[0].contains("No load needs the bump while the type is transient"));
        assert!(!breaks[0].contains("register the step"));
    }
    // Made persistent in the build: the step too.
    let breaks = breaks_between::<marker::Marker, marker_persistent::Marker>(false);
    assert_eq!(breaks.len(), 1, "{breaks:#?}");
    assert!(breaks[0].starts_with("`record_test::marker` changed its layout at version 1"));
    assert!(breaks[0].contains("register the step from 1"));
    assert!(breaks[0].contains("A transient type takes the bump too"));
    // Under a bump each change is one the record takes, and so is the policy's alone.
    assert!(breaks_between::<marker::Marker, marker_swapped_bumped::Marker>(false).is_empty());
    assert!(
        record::<marker_swapped_bumped::Marker, Handle>()
            .breaks(&record::<marker_persistent_bumped::Marker, Handle>(), false)
            .is_empty()
    );
}

/// The review's two-commit path: a transient type relaid at one version, the record
/// rewritten, then the type made persistent at that version, which every save written
/// before the relayout then fails to load. The relayout is refused, and the bump is the way
/// through.
#[test]
fn a_transient_relayout_is_refused_before_a_later_commit_makes_the_type_persistent() {
    let name = format!("schema_record_transient_{}.txt", std::process::id());
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_file(&path);
    let update =
        |registry: Registry| guard("test", &registry, &path, Mode::Update { renames: false });
    let one = |register: fn(&mut Registry) -> Result<()>| {
        let mut registry = Registry::new();
        register(&mut registry).unwrap();
        registry
    };

    update(one(Registry::register::<marker::Marker>)).unwrap();
    let written = std::fs::read_to_string(&path).unwrap();
    let refused = update(one(Registry::register::<marker_swapped::Marker>)).unwrap_err();
    assert!(refused.contains("`record_test::marker` changed its layout at version 1"), "{refused}");
    assert!(refused.contains("It is not rewritten."));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), written);
    // The commit that would make it persistent is refused against the same record.
    assert!(update(one(Registry::register::<marker_persistent::Marker>)).is_err());

    update(one(Registry::register::<marker_swapped_bumped::Marker>)).unwrap();
    update(one(Registry::register::<marker_persistent_bumped::Marker>)).unwrap();
    let registry = one(Registry::register::<marker_persistent_bumped::Marker>);
    guard("test", &registry, &path, Mode::Check).unwrap();
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn an_id_given_to_a_new_field_breaks_the_record_with_or_without_a_bump() {
    let digest = |record: Record| record.types[&pair::Pair::KEY].digest;
    assert_eq!(
        digest(record::<pair::Pair, Handle>()),
        digest(record::<pair_reused::Pair, Handle>())
    );
    for breaks in [
        breaks_between::<pair::Pair, pair_reused::Pair>(false),
        breaks_between::<pair::Pair, pair_reused_bumped::Pair>(false),
    ] {
        assert_eq!(breaks.len(), 1, "{breaks:#?}");
        assert!(breaks[0].starts_with(
            "field ID 2 of `record_test::pair` names `b` in the record and `c` in the build"
        ));
        assert!(breaks[0].contains("add 2 to `record_test::pair`'s `retired`"));
        assert!(breaks[0].contains(RENAME));
    }
}

#[test]
fn a_rename_breaks_the_record_unless_it_is_taken_as_one() {
    let breaks = breaks_between::<pair::Pair, pair_renamed::Pair>(false);
    assert_eq!(breaks.len(), 1, "{breaks:#?}");
    assert!(breaks[0].starts_with("field ID 1 of `record_test::pair` names `a`"));
    assert!(breaks_between::<pair::Pair, pair_renamed::Pair>(true).is_empty());
    // A rename does not excuse an ID that left the fields unretired.
    assert_eq!(breaks_between::<pair::Pair, pair_dropped::Pair>(true).len(), 1);
    // Nor a new name under an ID whose type changed: that is a new field the record can tell.
    for renames in [false, true] {
        let breaks = breaks_between::<pair::Pair, pair_retyped::Pair>(renames);
        assert_eq!(breaks.len(), 1, "{breaks:#?}");
        assert!(breaks[0].starts_with(
            "field ID 2 of `record_test::pair` names `b` in the record and `c` in the build"
        ));
        assert!(breaks[0].contains("two commits, the rename first"));
    }
}

#[test]
fn an_id_must_be_retired_when_its_field_leaves_and_stays_retired() {
    let breaks = breaks_between::<pair::Pair, pair_dropped::Pair>(false);
    assert_eq!(breaks.len(), 1, "{breaks:#?}");
    assert!(breaks[0].starts_with(
        "field ID 2 of `record_test::pair` (`b`) left the fields without being retired"
    ));
    assert!(breaks_between::<pair::Pair, pair_retired::Pair>(false).is_empty());
    let breaks = breaks_between::<pair_retired::Pair, pair_given_back::Pair>(false);
    assert_eq!(breaks.len(), 1, "{breaks:#?}");
    assert!(breaks[0].starts_with("field ID 2 of `record_test::pair` is retired in the record"));
}

#[test]
fn the_ids_of_a_nested_struct_are_held_like_a_types() {
    let breaks = breaks_between::<outer::Outer, outer_reused::Outer>(false);
    assert_eq!(breaks.len(), 2, "{breaks:#?}");
    assert!(breaks[0].starts_with(
        "field ID 1.2 of `record_test::outer` names `y` in the record and `z` in the build"
    ));
    assert!(breaks[0].contains("the `retired` of the `#[value]` struct field 1 holds"));
    assert!(breaks[1].starts_with("field ID 2.2 of `record_test::outer` names `y`"));
}

/// The review's nested case: a `#[value]` struct renamed in the commit that swaps two of its
/// fields' names, or gives one's ID to a new field, at one digest, which an older save then
/// loads with the values under the wrong names. A rename of the struct alone passes.
#[test]
fn a_renamed_struct_at_one_digest_keeps_its_ids() {
    let digest = |record: Record| record.types[&nest::Nest::KEY].digest;
    let before = digest(record::<nest::Nest, Handle>());
    assert_eq!(before, digest(record::<nest_renamed_swapped::Nest, Handle>()));
    assert_eq!(before, digest(record::<nest_renamed_reused::Nest, Handle>()));

    assert!(breaks_between::<nest::Nest, nest_renamed::Nest>(false).is_empty());
    let breaks = breaks_between::<nest::Nest, nest_renamed_swapped::Nest>(false);
    assert_eq!(breaks.len(), 2, "{breaks:#?}");
    assert!(breaks[0].starts_with(
        "field ID 1.1 of `record_test::nest` names `x` in the record and `y` in the build"
    ));
    assert!(breaks[1].starts_with("field ID 1.2 of `record_test::nest` names `y`"));
    let breaks = breaks_between::<nest::Nest, nest_renamed_reused::Nest>(false);
    assert_eq!(breaks.len(), 1, "{breaks:#?}");
    assert!(breaks[0].starts_with("field ID 1.2 of `record_test::nest` names `y`"));
}

#[test]
fn new_and_removed_types_change_the_record_and_derived_ones_stay_out() {
    let before = record::<pair::Pair, Handle>();
    assert_eq!(before.types.keys().collect::<Vec<_>>(), [&pair::Pair::KEY]);
    let after = record::<marker::Marker, Handle>();
    assert!(before.breaks(&after, false).is_empty());
    assert_eq!(
        before.changes(&after),
        ["`record_test::pair` is no longer saved", "`record_test::marker` is new"]
    );
}

#[test]
fn a_record_reads_back_as_written() {
    let types = ENGINE.types().unwrap();
    for record in
        [Record::of(types.registry()).unwrap(), record::<outer::Outer, pair_retired::Pair>()]
    {
        assert_eq!(Record::parse(&record.render("test")).unwrap(), record);
    }
    let text = record::<outer::Outer, Handle>().render("test");
    assert!(text.contains("\n  2 many: [struct Inner; 2]\n  2.1 x: u32\n  2.2 y: u32\n"), "{text}");
}

#[test]
fn a_record_with_crlf_line_endings_checks_as_written() {
    let types = ENGINE.types().unwrap();
    let text = Record::of(types.registry()).unwrap().render("engine");
    let name = format!("schema_record_crlf_{}.txt", std::process::id());
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, text.replace('\n', "\r\n")).unwrap();
    guard("engine", types.registry(), &path, Mode::Check).unwrap();
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn a_record_that_would_not_read_back_is_not_written() {
    let name = format!("schema_record_spaced_{}.txt", std::process::id());
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_file(&path);
    let mut registry = Registry::new();
    registry.register::<Spaced>().unwrap();
    let refused = guard("test", &registry, &path, Mode::Update { renames: false }).unwrap_err();
    assert!(refused.contains("would not read back") && refused.contains("not a type line"));
    assert!(!path.exists());
}

#[test]
fn a_record_that_cannot_be_read_is_refused() {
    let pair = record::<pair_retired::Pair, Handle>().render("test");
    let type_line = pair.lines().find(|line| line.starts_with("record_test::pair")).unwrap();
    for (text, what) in [
        ("  1 a: u32\n".to_owned(), "a field before any type"),
        (format!("{type_line}\n  1.1 a: u32\n"), "no such field"),
        (
            format!("{type_line}\n  1 a: u32\n  1.1 x: u32\n"),
            "an ID under a field that holds no struct",
        ),
        (format!("{type_line}\n  1 a: u32\n  retired 1\n"), "an ID both a field and retired"),
        (format!("{type_line}\n  retired 1\n  1 a: u32\n"), "an ID both a field and retired"),
        (format!("{type_line}\n  1 a: u32\n  1 b: u32\n"), "an ID stated twice"),
        (format!("{type_line}\n  retired 2\n  retired 2\n"), "an ID retired twice"),
        (format!("{type_line}\n  1 a u32\n"), "not a field line"),
        (format!("{type_line}\n{type_line}\n"), "a type stated twice"),
        (type_line.replace("record_test::pair", "record_test::paired"), "not a type line"),
        (format!("{type_line}\n<<<<<<< HEAD\n"), "not a type line"),
    ] {
        let error = Record::parse(&text).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(error.message().contains(what), "{what}: {error:?}");
    }
}

#[test]
fn the_guard_rewrites_only_what_a_deliberate_change_moved() {
    // One file per process: a runner may run this test beside itself.
    let name = format!("schema_record_guard_{}.txt", std::process::id());
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_file(&path);
    let registry = |swap: u32| {
        let mut registry = Registry::new();
        match swap {
            0 => registry.register::<pair::Pair>().unwrap(),
            1 => registry.register::<pair_swapped::Pair>().unwrap(),
            _ => registry.register::<pair_swapped_bumped::Pair>().unwrap(),
        }
        registry.register::<outer::Outer>().unwrap();
        registry
    };
    let check = |swap| guard("test", &registry(swap), &path, Mode::Check);
    let update = |swap| guard("test", &registry(swap), &path, Mode::Update { renames: false });

    let missing = check(0).unwrap_err();
    assert!(missing.contains("there is no record") && missing.contains(UPDATE), "{missing}");
    update(0).unwrap();
    check(0).unwrap();
    let written = std::fs::read_to_string(&path).unwrap();

    // A digest changed at one version: refused in both modes, the file untouched.
    for result in [check(1), update(1)] {
        let message = result.unwrap_err();
        assert!(message.contains("`record_test::pair` changed its layout at version 1"));
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), written);

    // The bump: out of date until rewritten, and the rewrite moves the type's line alone.
    let stale = check(2).unwrap_err();
    assert!(stale.contains("`record_test::pair` changed") && stale.contains(UPDATE), "{stale}");
    update(2).unwrap();
    check(2).unwrap();
    let rewritten = std::fs::read_to_string(&path).unwrap();
    let moved: Vec<_> =
        written.lines().zip(rewritten.lines()).filter(|(was, now)| was != now).collect();
    assert_eq!(written.lines().count(), rewritten.lines().count());
    assert_eq!(moved.len(), 1, "{moved:#?}");
    assert!(moved[0].1.starts_with("record_test::pair ") && moved[0].1.contains("version=2"));
    std::fs::remove_file(&path).unwrap();
}
