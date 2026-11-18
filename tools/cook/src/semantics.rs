//! Slang's reflection of the entry point it compiled to SPIR-V, read for what the module
//! cannot say: the semantic each varying and each fragment output is declared with, beside
//! the location Slang gave it (`README.md`, "The checks"). Metal links a varying and writes
//! a colour attachment by its semantic, Vulkan by its location, so each is refused unless
//! the two agree: `SV_Target<i>` and `TEXCOORD<i>` at location `i`, the index 0 when it is
//! not written.
//!
//! A struct's fields are at locations from the struct's, which the reflection gives each
//! relative to it.

use lactose_core::{Error, ErrorKind, Result};
use serde_json::Value;

/// What the reflection declares, each bit both a location and a semantic's index.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Semantics {
    /// Bit `i` for each `TEXCOORD<i>`, in either direction.
    pub varyings: u32,
    /// Bit `i` for each `SV_Target<i>`.
    pub outputs: u32,
}

/// Reads the entry point `entry` from the reflection `json` of a compile of it alone.
///
/// # Errors
/// `InvalidData` for JSON that is not a reflection of `entry` alone, for a varying or output
/// whose semantic's index is not its location, for a varying that is not `TEXCOORD<i>`, for
/// one that takes more than one location, and for a location named twice.
pub(crate) fn read(json: &[u8], entry: &str) -> Result<Semantics> {
    let reflection: Value = serde_json::from_slice(json)
        .map_err(|error| invalid(format!("Slang's reflection is not JSON: {error}")))?;
    let entries = reflection["entryPoints"].as_array().map_or(&[][..], Vec::as_slice);
    let [found] = entries else {
        return Err(invalid(format!(
            "Slang's reflection holds {} entry points, where it is of one",
            entries.len()
        )));
    };
    if found["name"].as_str() != Some(entry) {
        return Err(invalid(format!("Slang's reflection is not of the entry point {entry}")));
    }
    let mut semantics = Semantics { varyings: 0, outputs: 0 };
    let parameters = found["parameters"].as_array().map_or(&[][..], Vec::as_slice);
    for node in parameters.iter().chain(found.get("result")) {
        walk(node, 0, &mut semantics)?;
    }
    Ok(semantics)
}

fn invalid(message: impl Into<std::borrow::Cow<'static, str>>) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

/// Reads `node`, a parameter, the result or a field of either, whose location is counted
/// from `base`.
fn walk(node: &Value, base: u32, semantics: &mut Semantics) -> Result<()> {
    let name = node["name"].as_str().filter(|name| !name.is_empty());
    // `the varying `uv``, or `the returned fragment output` for an entry's result.
    let described = |what: &str| match name {
        Some(name) => format!("the {what} `{name}`"),
        None => format!("the returned {what}"),
    };
    let binding = &node["binding"];
    let varying = matches!(binding["kind"].as_str(), Some("varyingInput" | "varyingOutput"));
    let location = if varying {
        let index = binding["index"].as_u64().and_then(|index| u32::try_from(index).ok());
        let unread = || invalid(format!("{}'s location does not read", described("varying")));
        base.checked_add(index.ok_or_else(unread)?).ok_or_else(unread)?
    } else {
        base
    };
    let ty = &node["type"];
    if ty["kind"].as_str() == Some("struct") {
        for field in ty["fields"].as_array().map_or(&[][..], Vec::as_slice) {
            walk(field, location, semantics)?;
        }
        return Ok(());
    }
    let semantic = node["semanticName"].as_str().map(str::to_ascii_uppercase);
    let target = semantic.as_deref() == Some("SV_TARGET");
    let what = if target { "fragment output" } else { "varying" };
    match semantic.as_deref() {
        // A built-in, which has no location.
        Some(built_in) if built_in.starts_with("SV_") && !target => return Ok(()),
        // A parameter that is not a varying.
        None if !varying => return Ok(()),
        None => {
            return Err(invalid(format!(
                "{} has no semantic: declare it `TEXCOORD<i>`, since Metal links varyings by \
                 semantic",
                described(what)
            )));
        }
        Some(other) if !target && other != "TEXCOORD" => {
            return Err(invalid(format!(
                "{} is `{other}`, where each varying is `TEXCOORD<i>` at location i: Metal links \
                 varyings by semantic and Vulkan by location",
                described(what)
            )));
        }
        Some(_) => {}
    }
    let semantic = if target { "SV_Target" } else { "TEXCOORD" };
    if !varying {
        return Err(invalid(format!(
            "{}, `{semantic}`, has no location in Slang's reflection",
            described(what)
        )));
    }
    if !matches!(ty["kind"].as_str(), Some("scalar" | "vector")) {
        return Err(invalid(format!(
            "{}, `{semantic}`, takes more than one location: declare a scalar or a vector for \
             each semantic",
            described(what)
        )));
    }
    let index = node["semanticIndex"].as_u64().unwrap_or(0);
    if index != u64::from(location) {
        return Err(invalid(format!(
            "{} is `{semantic}{index}` at location {location}, where each is at its semantic's \
             index: declare it with `[[vk::location({index})]]`{}",
            described(what),
            if name.is_none() {
                ", since Slang places a returned value after any `out` outputs"
            } else {
                ""
            }
        )));
    }
    let bits = if target { &mut semantics.outputs } else { &mut semantics.varyings };
    if location >= u32::BITS || *bits & (1 << location) != 0 {
        return Err(invalid(format!("`{semantic}{index}` twice or out of range")));
    }
    *bits |= 1 << location;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reflection of `fs` whose one parameter is `parameter` and whose result is `result`.
    fn reflection(parameter: &str, result: &str) -> Vec<u8> {
        format!(r#"{{"entryPoints": [{{"name": "fs", "parameters": [{parameter}], "result": {result}}}]}}"#)
            .into_bytes()
    }

    const VECTOR: &str = r#"{"kind": "vector", "elementCount": 4}"#;

    fn leaf(
        name: &str,
        kind: &str,
        index: u32,
        semantic: &str,
        semantic_index: Option<u32>,
    ) -> String {
        let semantic_index =
            semantic_index.map(|i| format!(r#", "semanticIndex": {i}"#)).unwrap_or_default();
        format!(
            r#"{{"name": "{name}", "type": {VECTOR}, "binding": {{"kind": "{kind}", "index": {index}}}, "semanticName": "{semantic}"{semantic_index}}}"#
        )
    }

    fn fields(name: &str, kind: &str, index: u32, fields: &[String]) -> String {
        format!(
            r#"{{"name": "{name}", "binding": {{"kind": "{kind}", "index": {index}}}, "type": {{"kind": "struct", "fields": [{}]}}}}"#,
            fields.join(", ")
        )
    }

    #[test]
    fn each_varying_and_output_at_its_index_is_read() {
        let position =
            r#"{"name": "p", "type": {"kind": "vector"}, "semanticName": "SV_POSITION"}"#;
        let input = fields(
            "v",
            "varyingInput",
            0,
            &[
                position.to_owned(),
                leaf("uv", "varyingInput", 0, "TEXCOORD", None),
                fields(
                    "inner",
                    "varyingInput",
                    1,
                    &[leaf("x", "varyingInput", 0, "TEXCOORD", Some(1))],
                ),
            ],
        );
        let output =
            fields("", "varyingOutput", 0, &[leaf("c", "varyingOutput", 1, "SV_TARGET", Some(1))]);
        let read = read(&reflection(&input, &output), "fs").unwrap();
        assert_eq!(read, Semantics { varyings: 0b11, outputs: 0b10 });
    }

    #[test]
    fn an_output_or_varying_away_from_its_index_is_refused() {
        let none = r#"{"type": {"kind": "struct", "fields": []}}"#;
        for (parameter, result, says) in [
            // A returned value, at location 0.
            (none.to_owned(), leaf("", "varyingOutput", 0, "SV_TARGET", Some(1)), "is `SV_Target1` at location 0"),
            (none.to_owned(), leaf("c", "varyingOutput", 1, "SV_TARGET", None), "`c` is `SV_Target0` at location 1"),
            (leaf("uv", "varyingInput", 0, "TEXCOORD", Some(1)), none.to_owned(), "`uv` is `TEXCOORD1` at location 0"),
            // A field's location is its struct's and its own.
            (
                fields("v", "varyingInput", 1, &[leaf("x", "varyingInput", 0, "TEXCOORD", None)]),
                none.to_owned(),
                "`x` is `TEXCOORD0` at location 1",
            ),
            (leaf("n", "varyingInput", 0, "COLOR", None), none.to_owned(), "`n` is `COLOR`"),
            (
                r#"{"name": "n", "type": {"kind": "vector"}, "binding": {"kind": "varyingInput", "index": 0}}"#.to_owned(),
                none.to_owned(),
                "`n` has no semantic",
            ),
            (
                r#"{"name": "n", "type": {"kind": "vector"}, "semanticName": "TEXCOORD"}"#.to_owned(),
                none.to_owned(),
                "has no location in Slang's reflection",
            ),
            (
                leaf("m", "varyingInput", 0, "TEXCOORD", None).replace(VECTOR, r#"{"kind": "matrix"}"#),
                none.to_owned(),
                "takes more than one location",
            ),
            (
                fields("v", "varyingInput", 0, &[leaf("a", "varyingInput", 0, "TEXCOORD", None), leaf("b", "varyingInput", 0, "TEXCOORD", None)]),
                none.to_owned(),
                "`TEXCOORD0` twice",
            ),
        ] {
            let error = read(&reflection(&parameter, &result), "fs").unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
            assert!(error.message().contains(says), "{says}: {error}");
        }
    }

    #[test]
    fn the_hint_on_a_returned_value_alone_says_where_slang_places_it() {
        let none = r#"{"type": {"kind": "struct", "fields": []}}"#;
        let clause = "since Slang places a returned value after any `out` outputs";
        let returned = leaf("", "varyingOutput", 1, "SV_TARGET", None);
        let error = read(&reflection(none, &returned), "fs").unwrap_err();
        assert!(error.message().contains("declare it with `[[vk::location(0)]]`, "), "{error}");
        assert!(error.message().contains(clause), "{error}");
        // An `out` parameter, and a field of a returned struct.
        let out = leaf("b", "varyingOutput", 0, "SV_TARGET", Some(1));
        let field = fields("", "varyingOutput", 0, std::slice::from_ref(&out));
        for (parameter, result) in [(out.as_str(), none), (none, field.as_str())] {
            let error = read(&reflection(parameter, result), "fs").unwrap_err();
            assert!(error.message().ends_with("`[[vk::location(1)]]`"), "{error}");
        }
    }

    #[test]
    fn a_reflection_of_another_entry_or_of_two_is_refused() {
        let none = r#"{"type": {"kind": "struct", "fields": []}}"#;
        let error = read(&reflection(none, none), "vs").unwrap_err();
        assert!(error.message().contains("not of the entry point vs"), "{error}");
        let error = read(br#"{"entryPoints": []}"#, "fs").unwrap_err();
        assert!(error.message().contains("holds 0 entry points"), "{error}");
        let error =
            read(br#"{"entryPoints": [{"name": "fs"}, {"name": "fs"}]}"#, "fs").unwrap_err();
        assert!(error.message().contains("holds 2 entry points"), "{error}");
        let error = read(b"{", "fs").unwrap_err();
        assert!(error.message().contains("is not JSON"), "{error}");
    }
}
