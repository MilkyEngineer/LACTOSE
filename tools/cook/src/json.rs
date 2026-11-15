//! The JSON forms the reader accepts (`README.md`, "The glTF reader" and "The source
//! schema"): where the reader expects an object, an object only, and in the mark, no key
//! written twice in one object.
//!
//! serde's derive also takes a struct's fields as an array, by position, which would bypass
//! `deny_unknown_fields` and tie a source's meaning to the order of the fields' declarations;
//! and serde_json keeps the last of two equal keys without a word. The functions here are
//! the `deserialize_with` of every struct-typed field, and [`Extras`] reads a node's
//! `extras` so that a repeated key in the mark is a refusal.

use std::fmt;
use std::marker::PhantomData;

use serde::de::value::{MapAccessDeserializer, StringDeserializer};
use serde::de::{Deserialize, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};

/// A `T` written as a JSON object, and as nothing else.
pub(crate) fn object<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<T, D::Error> {
    Object::deserialize(deserializer).map(|Object(value)| value)
}

/// [`object`], boxed.
pub(crate) fn boxed<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Box<T>, D::Error> {
    object(deserializer).map(Box::new)
}

/// [`object`], or `null` for `None`. The field also needs `#[serde(default)]`, for `None`
/// when it is absent.
pub(crate) fn optional<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<Object<T>>::deserialize(deserializer).map(|value| value.map(|Object(value)| value))
}

/// A list of [`object`]s.
pub(crate) fn objects<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Vec<T>, D::Error> {
    Vec::<Object<T>>::deserialize(deserializer)
        .map(|values| values.into_iter().map(|Object(value)| value).collect())
}

/// A name: a `T` whose variants carry nothing, written as a JSON string and as nothing else.
/// serde would also take a variant written as a one-key object, `{"hips": null}`.
pub(crate) fn name<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<T, D::Error> {
    Name::deserialize(deserializer).map(|Name(value)| value)
}

/// [`name`], or `null` for `None`. The field also needs `#[serde(default)]`.
pub(crate) fn optional_name<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<Name<T>>::deserialize(deserializer).map(|value| value.map(|Name(value)| value))
}

/// An optional list of [`name`]s. The field also needs `#[serde(default)]`.
pub(crate) fn optional_names<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<Vec<T>>, D::Error> {
    Option::<Vec<Name<T>>>::deserialize(deserializer)
        .map(|values| values.map(|values| values.into_iter().map(|Name(value)| value).collect()))
}

struct Name<T>(T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Name<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        T::deserialize(StringDeserializer::<D::Error>::new(name)).map(Name)
    }
}

struct Object<T>(T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Fields<T>(PhantomData<T>);

        impl<'de, T: Deserialize<'de>> Visitor<'de> for Fields<T> {
            type Value = T;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON object")
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
                T::deserialize(MapAccessDeserializer::new(map))
            }
        }

        deserializer.deserialize_map(Fields(PhantomData)).map(Object)
    }
}

/// A node's `extras`, any JSON value as glTF allows: of it the reader keeps the value of
/// the one key it reads, and the path of the first key written twice in one object, at
/// that key or below it. Every other key is skipped unread.
#[derive(Default)]
pub(crate) struct Extras {
    /// The value under the key, if `extras` is an object that holds it.
    pub(crate) mark: Option<Value>,
    /// The first repeated key, as a path from `extras`, `lactose.bone.role` for instance.
    pub(crate) repeated: Option<String>,
}

/// The key of a node's `extras` object under which the source marks the node.
pub(crate) const MARK: &str = "lactose";

impl<'de> Deserialize<'de> for Extras {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ExtrasVisitor)
    }
}

struct ExtrasVisitor;

impl<'de> Visitor<'de> for ExtrasVisitor {
    type Value = Extras;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_bool<E>(self, _: bool) -> Result<Extras, E> {
        Ok(Extras::default())
    }

    fn visit_i64<E>(self, _: i64) -> Result<Extras, E> {
        Ok(Extras::default())
    }

    fn visit_u64<E>(self, _: u64) -> Result<Extras, E> {
        Ok(Extras::default())
    }

    fn visit_f64<E>(self, _: f64) -> Result<Extras, E> {
        Ok(Extras::default())
    }

    fn visit_str<E>(self, _: &str) -> Result<Extras, E> {
        Ok(Extras::default())
    }

    fn visit_unit<E>(self) -> Result<Extras, E> {
        Ok(Extras::default())
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Extras, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(Extras::default())
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Extras, A::Error> {
        let mut extras = Extras::default();
        while let Some(key) = map.next_key::<String>()? {
            if key != MARK {
                map.next_value::<IgnoredAny>()?;
                continue;
            }
            let Checked { value, repeated } = map.next_value()?;
            let repeated = match extras.mark.replace(value) {
                Some(_) => Some(MARK.to_owned()),
                None => repeated.map(|path| format!("{MARK}{path}")),
            };
            extras.repeated = extras.repeated.or(repeated);
        }
        Ok(extras)
    }
}

/// A JSON value read as serde_json's `Value` reads it, and the path below it of the first
/// key written twice in one object (`.a.b`, `[2]`), where `Value` keeps the last silently.
struct Checked {
    value: Value,
    repeated: Option<String>,
}

impl<'de> Deserialize<'de> for Checked {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(CheckedVisitor)
    }
}

struct CheckedVisitor;

impl CheckedVisitor {
    fn leaf<E>(value: Value) -> Result<Checked, E> {
        Ok(Checked { value, repeated: None })
    }
}

impl<'de> Visitor<'de> for CheckedVisitor {
    type Value = Checked;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Checked, E> {
        Self::leaf(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Checked, E> {
        Self::leaf(Value::from(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Checked, E> {
        Self::leaf(Value::from(value))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Checked, E> {
        // `Value`'s own reading: JSON holds no infinity or NaN, so this is never null.
        Self::leaf(Value::from(value))
    }

    fn visit_str<E>(self, value: &str) -> Result<Checked, E> {
        Self::leaf(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Checked, E> {
        Self::leaf(Value::String(value))
    }

    fn visit_unit<E>(self) -> Result<Checked, E> {
        Self::leaf(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Checked, A::Error> {
        let mut values = Vec::new();
        let mut first = None;
        while let Some(Checked { value, repeated }) = seq.next_element()? {
            if first.is_none() {
                first = repeated.map(|path| format!("[{}]{path}", values.len()));
            }
            values.push(value);
        }
        Ok(Checked { value: Value::Array(values), repeated: first })
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Checked, A::Error> {
        let mut values = Map::new();
        let mut first = None;
        while let Some(key) = map.next_key::<String>()? {
            let Checked { value, repeated } = map.next_value()?;
            if first.is_none() {
                first = if values.contains_key(&key) {
                    Some(format!(".{key}"))
                } else {
                    repeated.map(|path| format!(".{key}{path}"))
                };
            }
            values.insert(key, value);
        }
        Ok(Checked { value: Value::Object(values), repeated: first })
    }
}
