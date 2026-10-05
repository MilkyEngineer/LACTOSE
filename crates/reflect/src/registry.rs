//! The registry of every reflected component and resource type.

use core::any::TypeId;
use std::collections::BTreeMap;

use lactose_core::{ComponentKey, Error, ErrorKind, FixedHashMap, Result};

use crate::Reflect;
use crate::info::{Persistence, TypeInfo, TypeRole};

/// Components above this size draw a warning at registration (architecture section 6).
const LARGE_COMPONENT_BYTES: usize = 1024;

struct Entry {
    info: &'static TypeInfo,
    type_id: TypeId,
}

/// Every reflected type of one runtime, keyed by [`ComponentKey`].
///
/// A registry is an ordinary value, filled by explicit [`Registry::register`] calls from
/// functions each crate writes or generates. There is no global registry and nothing runs
/// before `main` (architecture section 18).
#[derive(Default)]
pub struct Registry {
    by_key: BTreeMap<ComponentKey, Entry>,
    by_type: FixedHashMap<TypeId, &'static TypeInfo>,
}

impl Registry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `T`. Registering the same type again is a no-op.
    ///
    /// The description is checked against the type instead of trusted: it fails if another
    /// type holds the same key (two names with one hash, or one name declared twice), if a
    /// field ID is missing or repeated, if the fields do not tile the type exactly, if the
    /// description's key, size or alignment contradict `T`, or if a transient type is not
    /// valid when zeroed.
    pub fn register<T: Reflect>(&mut self) -> Result<()> {
        let info = T::INFO;
        let name = info.name();
        let type_id = TypeId::of::<T>();
        let invalid = |message: String| Err(Error::new(ErrorKind::InvalidData, message));

        if let Some(existing) = self.by_key.get(&info.key()) {
            if existing.type_id == type_id {
                return Ok(());
            }
            let other = existing.info.name();
            return if other == name {
                invalid(format!("the name `{name}` is declared by two types"))
            } else {
                invalid(format!(
                    "`{name}` and `{other}` collide on key {:#018x}; rename one",
                    info.key().0
                ))
            };
        }

        if T::KEY != info.key() {
            return invalid(format!("`{name}`: the type's key is not the hash of its name"));
        }
        let layout = info.layout();
        if layout.size() != size_of::<T>() || layout.align() != align_of::<T>() {
            return invalid(format!(
                "`{name}`: described as {} bytes aligned to {}, but the type is {} aligned to {}",
                layout.size(),
                layout.align(),
                size_of::<T>(),
                align_of::<T>()
            ));
        }
        layout.check()?;
        if info.persistence() == Persistence::Transient
            && info.validate(&vec![0; layout.size()]).is_err()
        {
            return invalid(format!(
                "`{name}` is transient, so it is zeroed every tick, but its all-zero value is invalid"
            ));
        }

        if info.role() == TypeRole::Component && layout.size() > LARGE_COMPONENT_BYTES {
            tracing::warn!(
                component = name,
                size = layout.size(),
                "component is larger than 1 KiB; few rows fit a chunk"
            );
        }

        self.by_key.insert(info.key(), Entry { info, type_id });
        self.by_type.insert(type_id, info);
        Ok(())
    }

    /// The type registered under `key`.
    #[must_use]
    pub fn get(&self, key: ComponentKey) -> Option<&'static TypeInfo> {
        self.by_key.get(&key).map(|entry| entry.info)
    }

    /// The registered type whose Rust type is `type_id`.
    #[must_use]
    pub fn get_by_type(&self, type_id: TypeId) -> Option<&'static TypeInfo> {
        self.by_type.get(&type_id).copied()
    }

    /// Whether `T` is registered.
    #[must_use]
    pub fn contains<T: Reflect>(&self) -> bool {
        self.by_type.contains_key(&TypeId::of::<T>())
    }

    /// The number of registered types.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    /// Whether nothing is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }

    /// Every registered type in ascending key order, which is the same on every build,
    /// platform and registration order.
    pub fn iter(&self) -> impl Iterator<Item = &'static TypeInfo> + '_ {
        self.by_key.values().map(|entry| entry.info)
    }

    /// The schema as text, in [`Registry::iter`] order: one line per type with its role,
    /// name, key, version, policy and size, then one indented line per field with its ID
    /// (`-` for padding), name, kind and size. The text is a function of the registered
    /// types alone, so two builds can be compared by diffing it.
    #[must_use]
    pub fn dump(&self) -> String {
        let mut out = String::new();
        for info in self.iter() {
            info.dump(&mut out);
        }
        out
    }
}
