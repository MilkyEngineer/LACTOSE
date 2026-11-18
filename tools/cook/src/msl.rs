//! The MSL Slang emitted for one entry point, read back for what the SPIR-V of the same entry
//! must agree with: its colour inputs and outputs, its varyings and their types, its function
//! constants and their types, and its buffers (`README.md`, "The checks").
//!
//! The reader knows the form Slang writes an entry point in, `[[fragment]] <type> <name>(`
//! and its parameters on one line, and the structs it takes and returns with their fields
//! one to a line, and refuses MSL in which it does not find it rather than read nothing from
//! it.

use std::collections::BTreeSet;

use lactose_core::{Error, ErrorKind, Result};
use lactose_rhi_api::{ComponentType, ConstantSlot, ConstantType, ShaderStage, Varying};

/// The Metal buffer index of the push block.
const PUSH_BUFFER: u32 = 0;
/// The Metal buffer index of the heap's argument buffer, which `lactose.heap` declares.
const HEAP_BUFFER: u32 = 1;
/// The start of the type Slang gives `lactose.heap`'s parameter block.
const HEAP_TYPE: &str = "Heap_default_";

/// What the SPIR-V's reflection is compared with.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Interface {
    /// Bit `i` for each `[[color(i)]]` parameter of the entry point.
    pub inputs: u32,
    /// Each `[[user(TEXCOORD_i)]]` field, `TEXCOORD` for 0, at location `i`, by location: of
    /// the struct a vertex entry returns, or of the one a fragment entry takes as its stage
    /// input.
    pub varyings: Vec<Varying>,
    /// Bit `i` for each `[[color(i)]]` field of the struct it returns.
    pub outputs: u32,
    /// Each `constant <type> <name> [[function_constant(i)]]`, by index.
    pub constants: Vec<ConstantSlot>,
    /// Whether it takes the push block, at `[[buffer(0)]]`.
    pub push: bool,
    /// Whether it takes the heap's argument buffer, at `[[buffer(1)]]`.
    pub heap: bool,
}

/// Reads the entry point `entry` of `stage` from `source`.
///
/// # Errors
/// `InvalidData` if the entry point is not found once, a colour is named twice, a field has
/// two colours, a varying is not `TEXCOORD<i>` or not of a type the format holds, a function
/// constant is not, a buffer is off its index or taken twice, a texture or a sampler is bound
/// alone, or a struct it takes or returns is not one the source declares.
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
    for (index, line) in attributes(source, "function_constant") {
        let id = index?;
        let declared =
            line.trim().strip_prefix("constant ").and_then(|rest| rest.split(' ').next());
        let ty = match declared {
            Some("bool") => ConstantType::Bool,
            Some("int") => ConstantType::Int,
            Some("uint") => ConstantType::Uint,
            Some("float") => ConstantType::Float,
            _ => {
                return Err(invalid(format!(
                    "an MSL function constant of a type the format does not hold, where it \
                     holds bool, int, uint and float: {}",
                    line.trim()
                )));
            }
        };
        constants.push(ConstantSlot { id, ty });
    }
    constants.sort_unstable_by_key(|constant| constant.id);
    if constants.windows(2).any(|pair| pair[0].id == pair[1].id) {
        return Err(invalid("the MSL declares a function constant index twice"));
    }
    let at = line.find(&call).expect("found by the filter");
    let returned = line[qualifier.len()..at].trim();
    let list = parameters(&line[at + call.len()..]);
    let mut interface = Interface {
        inputs: 0,
        varyings: Vec::new(),
        outputs: 0,
        constants,
        push: false,
        heap: false,
    };
    for &parameter in &list {
        if parameter.contains("[[texture(") || parameter.contains("[[sampler(") {
            return Err(invalid(format!(
                "the entry takes a texture or a sampler bound alone, `{parameter}`: the heap is \
                 `lactose.heap`'s parameter block, one argument buffer at \
                 [[buffer({HEAP_BUFFER})]]"
            )));
        }
        let Some((index, _)) = attributes(parameter, "buffer").next() else { continue };
        let index = index?;
        let (expected, taken, what) = if parameter.starts_with(HEAP_TYPE) {
            (HEAP_BUFFER, &mut interface.heap, "the heap's argument buffer")
        } else {
            (PUSH_BUFFER, &mut interface.push, "the push block")
        };
        if index != expected || *taken {
            return Err(invalid(format!(
                "the entry takes {what} at [[buffer({index})]], where it is at \
                 [[buffer({expected})]] once: `lactose.heap` declares the heap at `register(b1)`, \
                 and every other buffer is reached through a device address"
            )));
        }
        *taken = true;
    }
    let declared = |name: &str| {
        struct_body(source, name).ok_or_else(|| {
            invalid(format!(
                "the entry takes or returns `{name}`, a struct the MSL does not declare"
            ))
        })
    };
    match stage {
        ShaderStage::Vertex if returned != "void" => {
            interface.varyings = varyings(declared(returned)?)?;
        }
        ShaderStage::Fragment => {
            for parameter in &list {
                interface.inputs |= colours(parameter, false)?;
            }
            let stage_in = list.iter().find(|parameter| parameter.ends_with("[[stage_in]]"));
            if let Some(name) = stage_in.and_then(|parameter| parameter.split(' ').next()) {
                interface.varyings = varyings(declared(name)?)?;
            }
            if returned != "void" {
                interface.outputs = colours(declared(returned)?, true)?;
            }
        }
        _ => {}
    }
    Ok(interface)
}

fn invalid(message: impl Into<std::borrow::Cow<'static, str>>) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

/// The parameters of a declaration whose text after the opening parenthesis is `text`, each
/// trimmed: split at its commas outside brackets, up to the parenthesis that closes it.
fn parameters(text: &str) -> Vec<&str> {
    let (mut found, mut depth, mut start) = (Vec::new(), 0usize, 0);
    for (at, char) in text.char_indices() {
        match char {
            '(' | '<' | '[' => depth += 1,
            ')' if depth == 0 => {
                found.push(&text[start..at]);
                break;
            }
            ')' | '>' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                found.push(&text[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    found.into_iter().map(str::trim).filter(|parameter| !parameter.is_empty()).collect()
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

/// The varyings the fields of `body` name, `[[user(TEXCOORD_i)]]` each, or
/// `[[user(TEXCOORD)]]` for 0, by location, each with the type its field is declared with.
fn varyings(body: &str) -> Result<Vec<Varying>> {
    let open = "[[user(";
    let mut bits = 0u32;
    let mut varyings = Vec::new();
    for line in body.lines() {
        let Some(at) = line.find(open) else { continue };
        let name = line[at + open.len()..].split_once(")]]").map_or("", |(name, _)| name);
        let index = match name.strip_prefix("TEXCOORD") {
            Some("") => Some(0),
            Some(rest) => rest.strip_prefix('_').and_then(|index| index.parse::<u32>().ok()),
            None => None,
        };
        let Some(index) = index.filter(|index| *index < u32::BITS) else {
            return Err(invalid(format!(
                "an MSL varying that is not `TEXCOORD<i>`: {}",
                line.trim()
            )));
        };
        if bits & (1 << index) != 0 {
            return Err(invalid(format!("MSL varying TEXCOORD_{index} twice")));
        }
        bits |= 1 << index;
        // The declaration's first word that is not an attribute such as `[[flat]]`.
        let declared = line[..at].split_whitespace().find(|word| !word.starts_with("[["));
        let (component, count) = declared.and_then(component_type).ok_or_else(|| {
            invalid(format!(
                "an MSL varying of a type the format does not hold, where it holds a float, an \
                 int or a uint, or a vector of two to four of them: {}",
                line.trim()
            ))
        })?;
        varyings.push(Varying { location: index, component, count });
    }
    varyings.sort_unstable_by_key(|varying| varying.location);
    Ok(varyings)
}

/// The component type and count of an MSL scalar or vector type: `float`, `int2`, `uint4`.
fn component_type(name: &str) -> Option<(ComponentType, u32)> {
    let (component, count) = [
        ("float", ComponentType::Float),
        ("uint", ComponentType::Uint),
        ("int", ComponentType::Int),
    ]
    .into_iter()
    .find_map(|(prefix, component)| name.strip_prefix(prefix).map(|count| (component, count)))?;
    match count {
        "" => Some((component, 1)),
        "2" | "3" | "4" => Some((component, count.parse().ok()?)),
        _ => None,
    }
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

struct pixelInput_0
{
    float2 uv_0 [[user(TEXCOORD)]];
    [[flat]] uint4 tint_0 [[user(TEXCOORD_2)]];
};

constant uint fc_TINT_0 [[function_constant(3)]];
[[fragment]] ROut_0 fs_resolve(pixelInput_0 _S1 [[stage_in]], float4 lit_0 [[color(0)]], float4 pos_0 [[position]], Push_0 constant* push_1 [[buffer(0)]], Heap_default_0 constant* heap_1 [[buffer(1)]])
{
";

    const VERTEX: &str = "\
struct vs_main_Result_0
{
    float4 position_0 [[position]];
    float2 uv_0 [[user(TEXCOORD_1)]];
};

[[vertex]] vs_main_Result_0 vs_main(uint vertex_0 [[vertex_id]], Push_0 constant* push_0 [[buffer(0)]])
{
";

    #[test]
    fn colours_varyings_constants_and_buffers_are_read() {
        let fragment = read(RESOLVE, ShaderStage::Fragment, "fs_resolve").unwrap();
        let expected = Interface {
            inputs: 0b1,
            varyings: vec![
                Varying { location: 0, component: ComponentType::Float, count: 2 },
                Varying { location: 2, component: ComponentType::Uint, count: 4 },
            ],
            outputs: 0b10,
            constants: vec![ConstantSlot { id: 3, ty: ConstantType::Uint }],
            push: true,
            heap: true,
        };
        assert_eq!(fragment, expected);
        let vertex = read(VERTEX, ShaderStage::Vertex, "vs_main").unwrap();
        let expected = Interface {
            inputs: 0,
            varyings: vec![Varying { location: 1, component: ComponentType::Float, count: 2 }],
            outputs: 0,
            constants: vec![],
            push: true,
            heap: false,
        };
        assert_eq!(vertex, expected);
    }

    #[test]
    fn an_entry_point_not_found_is_refused() {
        let error = read(RESOLVE, ShaderStage::Fragment, "fs_main").unwrap_err();
        assert!(error.message().contains("does not declare"), "{error}");
        let error = read(RESOLVE, ShaderStage::Vertex, "fs_resolve").unwrap_err();
        assert!(error.message().contains("does not declare"), "{error}");
    }

    #[test]
    fn an_entry_a_function_constant_or_a_colour_declared_twice_is_refused() {
        let entry = RESOLVE.lines().find(|line| line.starts_with("[[fragment]]")).unwrap();
        let source = format!("{RESOLVE}{entry}\n");
        let error = read(&source, ShaderStage::Fragment, "fs_resolve").unwrap_err();
        assert!(error.message().contains("does not declare"), "{error}");
        let source = RESOLVE.replace(
            "constant uint fc_TINT_0 [[function_constant(3)]];",
            "constant uint fc_TINT_0 [[function_constant(3)]];\n\
             constant bool fc_FLAG_0 [[function_constant(3)]];",
        );
        let error = read(&source, ShaderStage::Fragment, "fs_resolve").unwrap_err();
        assert!(error.message().contains("function constant index twice"), "{error}");
        let source =
            RESOLVE.replace("[[color(1)]];", "[[color(1)]];\n    float4 b_0 [[color(1)]];");
        let error = read(&source, ShaderStage::Fragment, "fs_resolve").unwrap_err();
        assert!(error.message().contains("colour 1 twice"), "{error}");
    }

    #[test]
    fn a_struct_taken_or_returned_that_is_not_declared_is_refused() {
        for (from, to) in
            [("struct ROut_0", "struct Other_0"), ("struct pixelInput_0", "struct P_0")]
        {
            let source = RESOLVE.replace(from, to);
            let error = read(&source, ShaderStage::Fragment, "fs_resolve").unwrap_err();
            assert!(error.message().contains("a struct the MSL does not declare"), "{error}");
        }
    }

    #[test]
    fn a_field_with_two_colours_is_refused() {
        let source = RESOLVE.replace("[[color(1)]];", "[[color(1)]] [[color(2)]];");
        let error = read(&source, ShaderStage::Fragment, "fs_resolve").unwrap_err();
        assert!(error.message().contains("two colours"), "{error}");
    }

    #[test]
    fn a_varying_that_is_not_texcoord_i_is_refused() {
        for (from, to, says) in [
            ("[[user(TEXCOORD_2)]]", "[[user(_SLANG_ATTR)]]", "not `TEXCOORD<i>`: [[flat]] uint4"),
            ("[[user(TEXCOORD_2)]]", "[[user(TEXCOORDS)]]", "not `TEXCOORD<i>`"),
            ("[[user(TEXCOORD_2)]]", "[[user(TEXCOORD_0)]]", "TEXCOORD_0 twice"),
        ] {
            let source = RESOLVE.replace(from, to);
            let error = read(&source, ShaderStage::Fragment, "fs_resolve").unwrap_err();
            assert!(error.message().contains(says), "{says}: {error}");
        }
        let source = VERTEX.replace("[[user(TEXCOORD_1)]]", "[[user(COLOR)]]");
        let error = read(&source, ShaderStage::Vertex, "vs_main").unwrap_err();
        assert!(error.message().contains("not `TEXCOORD<i>`"), "{error}");
    }

    #[test]
    fn each_type_is_read_and_one_the_format_does_not_hold_is_refused() {
        for (declared, expected) in [
            ("float", (ComponentType::Float, 1)),
            ("int3", (ComponentType::Int, 3)),
            ("uint2", (ComponentType::Uint, 2)),
        ] {
            let source = RESOLVE.replace("float2 uv_0", &format!("{declared} uv_0"));
            let fragment = read(&source, ShaderStage::Fragment, "fs_resolve").unwrap();
            let varying = fragment.varyings[0];
            assert_eq!(
                (varying.location, varying.component, varying.count),
                (0, expected.0, expected.1)
            );
        }
        for declared in ["half2", "float5", "bool", "float2x2", "short"] {
            let source = RESOLVE.replace("float2 uv_0", &format!("{declared} uv_0"));
            let error = read(&source, ShaderStage::Fragment, "fs_resolve").unwrap_err();
            let says = format!(
                "a type the format does not hold, where it holds a float, an int or a uint, or \
                 a vector of two to four of them: {declared} uv_0"
            );
            assert!(error.message().contains(&says), "{declared}: {error}");
        }
        for (declared, ty) in [
            ("bool", ConstantType::Bool),
            ("int", ConstantType::Int),
            ("float", ConstantType::Float),
        ] {
            let source = RESOLVE.replace("constant uint fc_", &format!("constant {declared} fc_"));
            let fragment = read(&source, ShaderStage::Fragment, "fs_resolve").unwrap();
            assert_eq!(fragment.constants, [ConstantSlot { id: 3, ty }]);
        }
        let source = RESOLVE.replace("constant uint fc_", "constant half fc_");
        let error = read(&source, ShaderStage::Fragment, "fs_resolve").unwrap_err();
        assert!(
            error.message().contains("function constant of a type the format does not hold"),
            "{error}"
        );
    }

    #[test]
    fn a_buffer_off_its_index_and_a_texture_or_sampler_alone_are_refused() {
        for (from, to, says) in [
            // The heap of a shader with no push block, before `register(b1)`.
            (
                "Push_0 constant* push_1 [[buffer(0)]], Heap_default_0 constant* heap_1 [[buffer(1)]]",
                "Heap_default_0 constant* heap_1 [[buffer(0)]]",
                "the heap's argument buffer at [[buffer(0)]], where it is at [[buffer(1)]]",
            ),
            ("push_1 [[buffer(0)]]", "push_1 [[buffer(1)]]", "the push block at [[buffer(1)]]"),
            (
                "Heap_default_0 constant* heap_1 [[buffer(1)]]",
                "Other_0 constant* other_1 [[buffer(0)]]",
                "the push block at [[buffer(0)]], where it is at [[buffer(0)]] once",
            ),
            (
                "Heap_default_0 constant* heap_1 [[buffer(1)]]",
                "array<texture2d<float, access::sample>, int(16)> images_1 [[texture(0)]]",
                "a texture or a sampler bound alone, `array<texture2d<float, access::sample>, \
                 int(16)> images_1 [[texture(0)]]`",
            ),
            (
                "Heap_default_0 constant* heap_1 [[buffer(1)]]",
                "array<sampler, int(4)> samplers_1 [[sampler(0)]]",
                "a texture or a sampler bound alone",
            ),
        ] {
            assert!(RESOLVE.contains(from), "{from}");
            let source = RESOLVE.replace(from, to);
            let error = read(&source, ShaderStage::Fragment, "fs_resolve").unwrap_err();
            assert!(error.message().contains(says), "{says}: {error}");
        }
    }

    #[test]
    fn parameters_split_outside_brackets() {
        assert_eq!(
            parameters(
                "array<texture2d<float, access::sample>, int(16)> a [[texture(0)]], uint b)\n{"
            ),
            ["array<texture2d<float, access::sample>, int(16)> a [[texture(0)]]", "uint b"],
        );
        assert_eq!(parameters(")"), Vec::<&str>::new());
    }
}
