//! The MSL Slang emitted for one entry point, read back for what the SPIR-V of the same entry
//! must agree with: its colour inputs and outputs and its function constants (`README.md`,
//! "The checks").
//!
//! The reader knows the form Slang writes an entry point in, `[[fragment]] <type> <name>(`
//! on one line and the returned struct's fields one to a line, and refuses MSL in which it
//! does not find it rather than read nothing from it.

use std::collections::BTreeSet;

use lactose_core::{Error, ErrorKind, Result};
use lactose_rhi_api::ShaderStage;

/// What the SPIR-V's reflection is compared with.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Interface {
    /// Bit `i` for each `[[color(i)]]` parameter of the entry point.
    pub inputs: u32,
    /// Bit `i` for each `[[color(i)]]` field of the struct it returns.
    pub outputs: u32,
    /// The index of each `[[function_constant(i)]]`, ascending.
    pub constants: Vec<u32>,
}

/// Reads the entry point `entry` of `stage` from `source`.
///
/// # Errors
/// `InvalidData` if the entry point is not found once, a colour is named twice, a field has
/// two colours, or the returned type is neither `void` nor a struct the source declares.
pub(crate) fn read(source: &str, stage: ShaderStage, entry: &str) -> Result<Interface> {
    let qualifier = match stage {
        ShaderStage::Vertex => "[[vertex]] ",
        ShaderStage::Fragment => "[[fragment]] ",
        ShaderStage::Compute => "[[kernel]] ",
    };
    let call = format!(" {entry}(");
    let mut found =
        source.lines().filter(|line| line.starts_with(qualifier) && line.contains(&call));
    let (Some(line), None) = (found.next(), found.next()) else {
        return Err(invalid(format!("the MSL does not declare `{qualifier}... {entry}(` once")));
    };
    let mut constants = Vec::new();
    for (index, _) in attributes(source, "function_constant") {
        constants.push(index?);
    }
    constants.sort_unstable();
    if constants.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(invalid("the MSL declares a function constant index twice"));
    }
    let (mut inputs, mut outputs) = (0, 0);
    if stage == ShaderStage::Fragment {
        let returned = line[qualifier.len()..line.find(&call).expect("found by the filter")].trim();
        let parameters = &line[line.find(&call).expect("found by the filter") + call.len()..];
        inputs = colours(parameters, false)?;
        if returned != "void" {
            let body = struct_body(source, returned).ok_or_else(|| {
                invalid(format!(
                    "the fragment entry returns `{returned}`, a struct the MSL does not declare"
                ))
            })?;
            outputs = colours(body, true)?;
        }
    }
    Ok(Interface { inputs, outputs, constants })
}

fn invalid(message: impl Into<std::borrow::Cow<'static, str>>) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

/// Each `[[<name>(<index>)]]` in `text`, with its index, and the line it is on.
fn attributes<'a>(text: &'a str, name: &str) -> impl Iterator<Item = (Result<u32>, &'a str)> {
    let open = format!("[[{name}(");
    text.lines().flat_map(move |line| {
        let mut found = Vec::new();
        let mut rest = line;
        while let Some(at) = rest.find(&open) {
            rest = &rest[at + open.len()..];
            let index = rest.split_once(")]]").and_then(|(index, _)| index.parse().ok());
            let index = index
                .ok_or_else(|| invalid(format!("an MSL attribute that does not read: {line}")));
            found.push((index, line));
        }
        found
    })
}

/// The bits of the colours `text` names; with `one_per_line`, each line names at most one.
fn colours(text: &str, one_per_line: bool) -> Result<u32> {
    let mut bits = 0u32;
    let mut lines = BTreeSet::new();
    for (index, line) in attributes(text, "color") {
        let index = index?;
        if one_per_line && !lines.insert(line.as_ptr()) {
            return Err(invalid(format!("an MSL output with two colours: {}", line.trim())));
        }
        if index >= u32::BITS || bits & (1 << index) != 0 {
            return Err(invalid(format!("MSL colour {index} twice or out of range")));
        }
        bits |= 1 << index;
    }
    Ok(bits)
}

/// The text between `struct <name>` and its closing `};`.
fn struct_body<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    let header = format!("struct {name}\n{{\n");
    let start = source.find(&header)? + header.len();
    let end = source[start..].find("\n};")?;
    Some(&source[start..start + end])
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESOLVE: &str = "\
struct ROut_0
{
    float4 resolved_0 [[color(1)]];
};

constant uint fc_TINT_0 [[function_constant(3)]];
[[fragment]] ROut_0 fs_resolve(pixelInput_0 _S1 [[stage_in]], float4 lit_0 [[color(0)]], float4 pos_0 [[position]])
{
";

    #[test]
    fn colours_and_constants_are_read() {
        let read = read(RESOLVE, ShaderStage::Fragment, "fs_resolve").unwrap();
        assert_eq!(read, Interface { inputs: 0b1, outputs: 0b10, constants: vec![3] });
    }

    #[test]
    fn an_entry_point_not_found_is_refused() {
        let error = read(RESOLVE, ShaderStage::Fragment, "fs_main").unwrap_err();
        assert!(error.message().contains("does not declare"), "{error}");
        let error = read(RESOLVE, ShaderStage::Vertex, "fs_resolve").unwrap_err();
        assert!(error.message().contains("does not declare"), "{error}");
    }

    #[test]
    fn a_returned_type_that_is_not_declared_is_refused() {
        let source = RESOLVE.replace("struct ROut_0", "struct Other_0");
        let error = read(&source, ShaderStage::Fragment, "fs_resolve").unwrap_err();
        assert!(error.message().contains("does not declare"), "{error}");
    }

    #[test]
    fn a_field_with_two_colours_is_refused() {
        let source = RESOLVE.replace("[[color(1)]];", "[[color(1)]] [[color(2)]];");
        let error = read(&source, ShaderStage::Fragment, "fs_resolve").unwrap_err();
        assert!(error.message().contains("two colours"), "{error}");
    }
}
