//! The SPIR-V module Slang emitted for one entry point, read back: what the cooked format
//! carries of it, and the checks decision 5.8 puts on the output rather than on the source
//! (`README.md`, "The checks").
//!
//! The reader knows the instructions the checks need and skips every other one by its word
//! count, so a module that uses more of SPIR-V than the engine's shaders do still reads. What
//! it cannot account for, it refuses: a check that cannot see an access lets nothing through.

use std::collections::{BTreeMap, BTreeSet};

use lactose_core::{Error, ErrorKind, Result};
use lactose_rhi_api::{
    ConstantSlot, HEAP_SET, HeapArray, HeapArrayKind, HeapForm, HeapUse, PUSH_BYTES_MAX,
    ShaderStage,
};

const MAGIC: u32 = 0x0723_0203;

// Opcodes.
const OP_EXTENSION: u16 = 10;
const OP_ENTRY_POINT: u16 = 15;
const OP_CAPABILITY: u16 = 17;
const OP_TYPE_BOOL: u16 = 20;
const OP_TYPE_INT: u16 = 21;
const OP_TYPE_FLOAT: u16 = 22;
const OP_TYPE_VECTOR: u16 = 23;
const OP_TYPE_MATRIX: u16 = 24;
const OP_TYPE_IMAGE: u16 = 25;
const OP_TYPE_SAMPLER: u16 = 26;
const OP_TYPE_ARRAY: u16 = 28;
const OP_TYPE_RUNTIME_ARRAY: u16 = 29;
const OP_TYPE_STRUCT: u16 = 30;
const OP_TYPE_POINTER: u16 = 32;
const OP_CONSTANT: u16 = 43;
const OP_SPEC_CONSTANT_TRUE: u16 = 48;
const OP_SPEC_CONSTANT_FALSE: u16 = 49;
const OP_SPEC_CONSTANT: u16 = 50;
const OP_SPEC_CONSTANT_COMPOSITE: u16 = 51;
const OP_SPEC_CONSTANT_OP: u16 = 52;
const OP_FUNCTION_CALL: u16 = 57;
const OP_VARIABLE: u16 = 59;
const OP_LOAD: u16 = 61;
const OP_STORE: u16 = 62;
const OP_COPY_MEMORY: u16 = 63;
const OP_COPY_MEMORY_SIZED: u16 = 64;
const OP_ACCESS_CHAIN: u16 = 65;
const OP_IN_BOUNDS_ACCESS_CHAIN: u16 = 66;
const OP_PTR_ACCESS_CHAIN: u16 = 67;
const OP_IN_BOUNDS_PTR_ACCESS_CHAIN: u16 = 70;
const OP_DECORATE: u16 = 71;
const OP_MEMBER_DECORATE: u16 = 72;
const OP_COPY_OBJECT: u16 = 83;
const OP_CONVERT_PTR_TO_U: u16 = 117;
const OP_BITCAST: u16 = 124;
const OP_SELECT: u16 = 169;
const OP_PHI: u16 = 245;

// Decorations.
const SPEC_ID: u32 = 1;
const ARRAY_STRIDE: u32 = 6;
const MATRIX_STRIDE: u32 = 7;
const BUILT_IN: u32 = 11;
const LOCATION: u32 = 30;
const BINDING: u32 = 33;
const DESCRIPTOR_SET: u32 = 34;
const OFFSET: u32 = 35;
const INPUT_ATTACHMENT_INDEX: u32 = 43;
const NON_UNIFORM: u32 = 5300;

// Storage classes.
const UNIFORM_CONSTANT: u32 = 0;
const UNIFORM: u32 = 2;
const OUTPUT: u32 = 3;
const PUSH_CONSTANT: u32 = 9;
const STORAGE_BUFFER: u32 = 12;
const PHYSICAL_STORAGE_BUFFER: u32 = 5349;

// Image dimensions.
const DIM_2D: u32 = 1;
const DIM_SUBPASS_DATA: u32 = 6;

/// The capabilities `rhi-api`'s README lists, each of which the required set enables, by
/// number and name.
const ALLOWED: [(u32, &str); 7] = [
    (1, "Shader"),
    (11, "Int64"),
    (40, "InputAttachment"),
    (55, "StorageImageReadWithoutFormat"),
    (5301, "ShaderNonUniform"),
    (5307, "SampledImageArrayNonUniformIndexing"),
    (5347, "PhysicalStorageBufferAddresses"),
];

/// The extension the list's `PhysicalStorageBufferAddresses` needs, and the only one.
const EXTENSION: &str = "SPV_KHR_physical_storage_buffer";

/// What a refused capability most often means, for the three the README names.
const REFUSED: [(u32, &str, &str); 3] = [
    (
        4427,
        "DrawParameters",
        "`SV_VertexID` or `SV_InstanceID` declares it: read `SV_VulkanVertexID` and \
         `SV_VulkanInstanceID`, whose ids are the same on both targets",
    ),
    (
        5302,
        "RuntimeDescriptorArray",
        "a runtime descriptor array, such as Slang's own heap behind `DescriptorHandle`, \
         declares it: reach the heap through `lactose.heap`'s fixed arrays",
    ),
    (
        12,
        "Int64Atomics",
        "64-bit atomics declare it, and the renderer selects them from milestone 3",
    ),
];

/// What the cooked format carries of one entry point's module.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Reflection {
    pub stage: ShaderStage,
    pub entry: String,
    pub heap: Option<HeapUse>,
    pub constants: Vec<ConstantSlot>,
    pub push_bytes: u32,
    pub inputs: u32,
    pub outputs: u32,
}

/// The names of a module's entry points, in the order the module declares them.
pub(crate) fn entry_points(code: &[u8]) -> Result<Vec<String>> {
    let module = Module::parse(code)?;
    Ok(module.entries.into_iter().map(|entry| entry.name).collect())
}

/// Reads a module of one entry point and runs every check on it.
///
/// # Errors
/// `InvalidData` for bytes that are not a SPIR-V module and for every check the module
/// fails, the message naming the check.
pub(crate) fn reflect(code: &[u8]) -> Result<Reflection> {
    let module = Module::parse(code)?;
    module.check_capabilities()?;
    let [entry] = module.entries.as_slice() else {
        return Err(invalid(format!("{} entry points, where one is cooked", module.entries.len())));
    };
    let stage = match entry.model {
        0 => ShaderStage::Vertex,
        4 => ShaderStage::Fragment,
        5 => ShaderStage::Compute,
        model => return Err(invalid(format!("execution model {model}, which is not cooked"))),
    };
    let descriptors = module.descriptors(stage)?;
    module.check_heap_accesses(&descriptors.heap)?;
    let outputs = if stage == ShaderStage::Fragment { module.outputs(entry)? } else { 0 };
    Ok(Reflection {
        stage,
        entry: entry.name.clone(),
        heap: descriptors.heap_use(),
        constants: module.constants()?,
        push_bytes: module.push_bytes()?,
        inputs: descriptors.inputs,
        outputs,
    })
}

fn invalid(message: impl Into<std::borrow::Cow<'static, str>>) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

#[derive(Clone, Debug)]
enum Type {
    Bool,
    Int { width: u32 },
    Float { width: u32 },
    Vector { component: u32, count: u32 },
    Matrix { column: u32, count: u32 },
    Image { dim: u32 },
    Sampler,
    Array { element: u32, length: u32 },
    RuntimeArray,
    Struct { members: Vec<u32> },
    Pointer { storage: u32, pointee: u32 },
    Other,
}

struct Entry {
    model: u32,
    name: String,
    interface: Vec<u32>,
}

/// One instruction: its opcode and its operands, the word count's word left out.
struct Instruction {
    op: u16,
    operands: Vec<u32>,
}

/// Each decoration of an id or a member: the decoration and its literal operands.
type Decorations = Vec<(u32, Vec<u32>)>;

#[derive(Default)]
struct Module {
    capabilities: Vec<u32>,
    extensions: Vec<String>,
    entries: Vec<Entry>,
    decorations: BTreeMap<u32, Decorations>,
    member_decorations: BTreeMap<(u32, u32), Decorations>,
    types: BTreeMap<u32, Type>,
    constants: BTreeMap<u32, u32>,
    spec_constants: BTreeMap<u32, (u16, u32)>,
    variables: BTreeMap<u32, (u32, u32)>,
    /// Every instruction inside a function body, in order.
    body: Vec<Instruction>,
}

impl Module {
    fn parse(code: &[u8]) -> Result<Self> {
        if !code.len().is_multiple_of(4) || code.len() < 20 {
            return Err(invalid("not a SPIR-V module: its length is not a header and whole words"));
        }
        let words: Vec<u32> =
            code.as_chunks::<4>().0.iter().map(|w| u32::from_le_bytes(*w)).collect();
        if words[0] != MAGIC {
            return Err(invalid("not a little-endian SPIR-V module: its magic number is wrong"));
        }
        let mut module = Self::default();
        let mut at = 5;
        let mut in_function = false;
        while at < words.len() {
            let count = (words[at] >> 16) as usize;
            let op = (words[at] & 0xffff) as u16;
            if count == 0 || at + count > words.len() {
                return Err(invalid(format!("an instruction at word {at} runs past the module")));
            }
            let operands = &words[at + 1..at + count];
            at += count;
            module.read(op, operands, &mut in_function)?;
        }
        Ok(module)
    }

    fn read(&mut self, op: u16, operands: &[u32], in_function: &mut bool) -> Result<()> {
        let short = || invalid(format!("opcode {op} has too few operands"));
        let word = |i: usize| operands.get(i).copied().ok_or_else(short);
        match op {
            // OpFunction and OpFunctionEnd.
            54 => *in_function = true,
            56 => *in_function = false,
            _ if *in_function => {
                self.body.push(Instruction { op, operands: operands.to_vec() });
            }
            OP_CAPABILITY => self.capabilities.push(word(0)?),
            OP_EXTENSION => self.extensions.push(string(operands).0),
            OP_ENTRY_POINT => {
                let (name, used) = string(operands.get(2..).ok_or_else(short)?);
                let interface = operands.get(2 + used..).unwrap_or_default().to_vec();
                self.entries.push(Entry { model: word(0)?, name, interface });
            }
            OP_DECORATE => {
                let decoration = (word(1)?, operands.get(2..).unwrap_or_default().to_vec());
                self.decorations.entry(word(0)?).or_default().push(decoration);
            }
            OP_MEMBER_DECORATE => {
                let decoration = (word(2)?, operands.get(3..).unwrap_or_default().to_vec());
                let member = (word(0)?, word(1)?);
                self.member_decorations.entry(member).or_default().push(decoration);
            }
            OP_TYPE_BOOL => _ = self.types.insert(word(0)?, Type::Bool),
            OP_TYPE_INT => _ = self.types.insert(word(0)?, Type::Int { width: word(1)? }),
            OP_TYPE_FLOAT => _ = self.types.insert(word(0)?, Type::Float { width: word(1)? }),
            OP_TYPE_VECTOR => {
                let vector = Type::Vector { component: word(1)?, count: word(2)? };
                self.types.insert(word(0)?, vector);
            }
            OP_TYPE_MATRIX => {
                let matrix = Type::Matrix { column: word(1)?, count: word(2)? };
                self.types.insert(word(0)?, matrix);
            }
            // The sampled type, then the dimension.
            OP_TYPE_IMAGE => _ = self.types.insert(word(0)?, Type::Image { dim: word(2)? }),
            OP_TYPE_SAMPLER => _ = self.types.insert(word(0)?, Type::Sampler),
            OP_TYPE_ARRAY => {
                let array = Type::Array { element: word(1)?, length: word(2)? };
                self.types.insert(word(0)?, array);
            }
            OP_TYPE_RUNTIME_ARRAY => _ = self.types.insert(word(0)?, Type::RuntimeArray),
            OP_TYPE_STRUCT => {
                let members = operands.get(1..).unwrap_or_default().to_vec();
                self.types.insert(word(0)?, Type::Struct { members });
            }
            OP_TYPE_POINTER => {
                let pointer = Type::Pointer { storage: word(1)?, pointee: word(2)? };
                self.types.insert(word(0)?, pointer);
            }
            // Every other type the reader keeps by its result alone, so a member, an element
            // or a pointee of it is known to be a type the checks do not size.
            19 | 27 | 31 | 33 | 34 | 35 | 36 | 37 | 38 | 322 | 327 | 4472 | 5341 => {
                self.types.insert(word(0)?, Type::Other);
            }
            // A 32-bit value is all a length or a size needs; a wider one keeps its low word.
            OP_CONSTANT => _ = self.constants.insert(word(1)?, word(2)?),
            OP_SPEC_CONSTANT_TRUE | OP_SPEC_CONSTANT_FALSE | OP_SPEC_CONSTANT => {
                self.spec_constants.insert(word(1)?, (op, word(0)?));
            }
            OP_SPEC_CONSTANT_COMPOSITE | OP_SPEC_CONSTANT_OP => {
                self.spec_constants.insert(word(1)?, (op, word(0)?));
            }
            OP_VARIABLE => _ = self.variables.insert(word(1)?, (word(0)?, word(2)?)),
            _ => {}
        }
        Ok(())
    }

    fn decorations(&self, id: u32, decoration: u32) -> impl Iterator<Item = &[u32]> {
        let all = self.decorations.get(&id).map_or(&[][..], Vec::as_slice);
        all.iter().filter(move |(d, _)| *d == decoration).map(|(_, operands)| operands.as_slice())
    }

    fn decoration(&self, id: u32, decoration: u32) -> Option<u32> {
        self.decorations(id, decoration).next().and_then(|operands| operands.first().copied())
    }

    fn member_decoration(&self, member: (u32, u32), decoration: u32) -> Option<u32> {
        let all = self.member_decorations.get(&member)?;
        let found = all.iter().find(|(d, _)| *d == decoration)?;
        found.1.first().copied()
    }

    fn is(&self, id: u32, decoration: u32) -> bool {
        self.decorations(id, decoration).next().is_some()
    }

    fn check_capabilities(&self) -> Result<()> {
        for &capability in &self.capabilities {
            if ALLOWED.iter().any(|(allowed, _)| *allowed == capability) {
                continue;
            }
            let Some((_, name, why)) = REFUSED.iter().find(|(refused, ..)| *refused == capability)
            else {
                return Err(invalid(format!(
                    "the module declares capability {capability}, which is outside rhi-api's list"
                )));
            };
            return Err(invalid(format!(
                "the module declares capability {name}, which is outside rhi-api's list: {why}"
            )));
        }
        if let Some(other) = self.extensions.iter().find(|name| *name != EXTENSION) {
            return Err(invalid(format!(
                "the module declares extension {other}, where {EXTENSION} is the only one rhi-api \
                 lists"
            )));
        }
        Ok(())
    }

    /// The pointee of a variable's pointer type, and its storage class.
    fn pointee(&self, variable: u32) -> Result<(u32, u32)> {
        let (pointer, storage) = self.variables[&variable];
        match self.types.get(&pointer) {
            Some(Type::Pointer { pointee, .. }) => Ok((*pointee, storage)),
            _ => Err(invalid(format!("variable %{variable} has a type that is not a pointer"))),
        }
    }

    /// Every descriptor the module declares: the heap's arrays and the input attachments,
    /// and nothing else.
    fn descriptors(&self, stage: ShaderStage) -> Result<Descriptors> {
        let mut found = Descriptors::default();
        for &variable in self.variables.keys() {
            let (pointee, storage) = self.pointee(variable)?;
            if storage == UNIFORM || storage == STORAGE_BUFFER {
                return Err(invalid(format!(
                    "a uniform or storage buffer, %{variable}: a pipeline's layout is the heap's \
                     block, the input attachments and one push-constant range, and a buffer is \
                     reached through a device address"
                )));
            }
            if storage != UNIFORM_CONSTANT {
                continue;
            }
            let (Some(set), Some(binding)) =
                (self.decoration(variable, DESCRIPTOR_SET), self.decoration(variable, BINDING))
            else {
                return Err(invalid(format!("a resource, %{variable}, with no set and binding")));
            };
            let ty = self.types.get(&pointee);
            match (set, binding, ty) {
                (HEAP_SET, 0 | 1, Some(Type::Array { element, length })) => {
                    let kind = match (binding, self.types.get(element)) {
                        (0, Some(Type::Image { dim: DIM_2D })) => HeapArrayKind::SampledImage2d,
                        (1, Some(Type::Sampler)) => HeapArrayKind::Sampler,
                        _ => {
                            return Err(invalid(format!(
                                "set {HEAP_SET}, binding {binding} is the heap's {}, and the \
                                 module declares another type there",
                                if binding == 0 { "images" } else { "samplers" }
                            )));
                        }
                    };
                    let Some(&capacity) = self.constants.get(length) else {
                        return Err(invalid(format!(
                            "the heap's array at binding {binding} has a length that is not a \
                             constant"
                        )));
                    };
                    found.heap.insert(variable, HeapArray { kind, binding, capacity });
                }
                (HEAP_SET, ..) => {
                    return Err(invalid(format!(
                        "set {HEAP_SET} is the heap's, and binding {binding} there is not one of \
                         its fixed arrays (images at 0, samplers at 1){}",
                        if matches!(ty, Some(Type::RuntimeArray)) {
                            ": it is a runtime array, which `DescriptorHandle` declares"
                        } else {
                            ""
                        }
                    )));
                }
                (0, _, Some(Type::Image { dim: DIM_SUBPASS_DATA })) => {
                    if stage != ShaderStage::Fragment {
                        return Err(invalid("an input attachment outside the fragment stage"));
                    }
                    let index = self.decoration(variable, INPUT_ATTACHMENT_INDEX);
                    if index != Some(binding) || binding >= u32::BITS {
                        return Err(invalid(format!(
                            "an input attachment at binding {binding} whose index is {index:?}: \
                             input i is at binding i of set 0"
                        )));
                    }
                    found.inputs |= 1 << binding;
                }
                _ => {
                    return Err(invalid(format!(
                        "a resource at set {set}, binding {binding}, which is neither the heap's \
                         nor an input attachment"
                    )));
                }
            }
        }
        let mut kinds: Vec<HeapArrayKind> = found.heap.values().map(|array| array.kind).collect();
        kinds.sort_by_key(|kind| *kind as u8);
        if kinds.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(invalid("the heap's array of one kind is declared twice"));
        }
        Ok(found)
    }

    /// Every access through the heap carries the non-uniform decoration: each access chain
    /// into one of its arrays, and each load through one. A use of the array itself, rather
    /// than of an element reached by an access chain, is refused: the check would not see
    /// the access.
    fn check_heap_accesses(&self, heap: &BTreeMap<u32, HeapArray>) -> Result<()> {
        let mut reached: BTreeSet<u32> = BTreeSet::new();
        for instruction in &self.body {
            let operands = &instruction.operands;
            let through = |id: &u32| heap.contains_key(id) || reached.contains(id);
            match instruction.op {
                OP_ACCESS_CHAIN
                | OP_IN_BOUNDS_ACCESS_CHAIN
                | OP_PTR_ACCESS_CHAIN
                | OP_IN_BOUNDS_PTR_ACCESS_CHAIN
                    if operands.get(2).is_some_and(through) =>
                {
                    let result = operands[1];
                    if !self.is(result, NON_UNIFORM) {
                        return Err(invalid(format!(
                            "an access through the heap, %{result}, carries no non-uniform \
                             decoration: index the heap with `lactose.heap`'s marker, called \
                             where the index is used so that it is inlined"
                        )));
                    }
                    reached.insert(result);
                }
                OP_LOAD if operands.get(2).is_some_and(|id| heap.contains_key(id)) => {
                    return Err(invalid("the heap's array is loaded whole"));
                }
                OP_LOAD if operands.get(2).is_some_and(|id| reached.contains(id)) => {
                    let result = operands[1];
                    if !self.is(result, NON_UNIFORM) {
                        return Err(invalid(format!(
                            "a load through the heap, %{result}, carries no non-uniform \
                             decoration"
                        )));
                    }
                }
                OP_STORE | OP_COPY_MEMORY | OP_COPY_MEMORY_SIZED
                    if operands.iter().take(2).any(through) =>
                {
                    return Err(invalid("the heap is written or copied through a pointer"));
                }
                OP_FUNCTION_CALL | OP_COPY_OBJECT | OP_CONVERT_PTR_TO_U | OP_BITCAST
                | OP_SELECT | OP_PHI
                    if operands.iter().skip(2).any(through) =>
                {
                    return Err(invalid(format!(
                        "a pointer into the heap is passed on (opcode {}), where the check can \
                         follow an access chain alone",
                        instruction.op
                    )));
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Bit `i` for each fragment output at location `i`, each with one location.
    fn outputs(&self, entry: &Entry) -> Result<u32> {
        let mut outputs = 0;
        for &variable in &entry.interface {
            let Some(&(_, storage)) = self.variables.get(&variable) else { continue };
            if storage != OUTPUT || self.is(variable, BUILT_IN) {
                continue;
            }
            let (pointee, _) = self.pointee(variable)?;
            let locations: Vec<u32> =
                self.decorations(variable, LOCATION).filter_map(|o| o.first().copied()).collect();
            let [location] = locations.as_slice() else {
                return Err(invalid(format!(
                    "a fragment output, %{variable}, with {} locations, where each has one",
                    locations.len()
                )));
            };
            if matches!(self.types.get(&pointee), Some(Type::Array { .. } | Type::Struct { .. })) {
                return Err(invalid(format!(
                    "the fragment output at location {location} is an array or a struct, which \
                     takes more than one location"
                )));
            }
            if *location >= u32::BITS || outputs & (1 << location) != 0 {
                return Err(invalid(format!(
                    "fragment output location {location} twice or out of range"
                )));
            }
            outputs |= 1 << location;
        }
        Ok(outputs)
    }

    /// The specialisation constants, in ascending id order.
    fn constants(&self) -> Result<Vec<ConstantSlot>> {
        let mut constants = Vec::new();
        for (&id, &(op, ty)) in &self.spec_constants {
            let Some(slot) = self.decoration(id, SPEC_ID) else {
                if op == OP_SPEC_CONSTANT_COMPOSITE || op == OP_SPEC_CONSTANT_OP {
                    continue;
                }
                return Err(invalid(format!("a specialisation constant, %{id}, with no id")));
            };
            let bytes = match self.types.get(&ty) {
                Some(Type::Bool) => 1,
                Some(Type::Int { width: 32 } | Type::Float { width: 32 }) => 4,
                _ => {
                    return Err(invalid(format!(
                        "specialisation constant {slot} is neither a boolean nor 32 bits wide"
                    )));
                }
            };
            constants.push(ConstantSlot { id: slot, bytes });
        }
        constants.sort_by_key(|constant| constant.id);
        if constants.windows(2).any(|pair| pair[0].id == pair[1].id) {
            return Err(invalid("two specialisation constants share an id"));
        }
        Ok(constants)
    }

    /// The bytes of the push-constant block: the end of its last member.
    fn push_bytes(&self) -> Result<u32> {
        let mut blocks = Vec::new();
        for &variable in self.variables.keys() {
            let (pointee, storage) = self.pointee(variable)?;
            if storage == PUSH_CONSTANT {
                blocks.push(pointee);
            }
        }
        let bytes = match blocks.as_slice() {
            [] => 0,
            [block] => self.size(*block, None)?,
            _ => return Err(invalid("more than one push-constant block")),
        };
        if bytes > PUSH_BYTES_MAX {
            return Err(invalid(format!(
                "a push-constant block of {bytes} bytes, above PUSH_BYTES_MAX ({PUSH_BYTES_MAX})"
            )));
        }
        Ok(bytes)
    }

    /// The bytes a value of type `ty` takes in a block; `member` is the struct member it is,
    /// for a matrix's stride.
    fn size(&self, ty: u32, member: Option<(u32, u32)>) -> Result<u32> {
        let unsized_ = || {
            invalid(format!("the push-constant block holds type %{ty}, which the cook cannot size"))
        };
        let size = match self.types.get(&ty).ok_or_else(unsized_)? {
            Type::Int { width } | Type::Float { width } => width / 8,
            Type::Vector { component, count } => self.size(*component, None)? * count,
            Type::Matrix { column, count } => {
                let stride = member.and_then(|m| self.member_decoration(m, MATRIX_STRIDE));
                let column = match stride {
                    Some(stride) => stride,
                    None => self.size(*column, None)?,
                };
                column * count
            }
            Type::Array { length, .. } => {
                let stride = self.decoration(ty, ARRAY_STRIDE).ok_or_else(unsized_)?;
                stride * self.constants.get(length).ok_or_else(unsized_)?
            }
            Type::Struct { members } => {
                let mut end = 0;
                for (index, &member_ty) in (0u32..).zip(members) {
                    let at = (ty, index);
                    let offset = self.member_decoration(at, OFFSET).ok_or_else(unsized_)?;
                    end = end.max(offset + self.size(member_ty, Some(at))?);
                }
                end
            }
            Type::Pointer { storage: PHYSICAL_STORAGE_BUFFER, .. } => 8,
            _ => return Err(unsized_()),
        };
        Ok(size)
    }
}

/// The descriptors a module declares.
#[derive(Default)]
struct Descriptors {
    /// The heap's arrays, by variable.
    heap: BTreeMap<u32, HeapArray>,
    inputs: u32,
}

impl Descriptors {
    fn heap_use(&self) -> Option<HeapUse> {
        if self.heap.is_empty() {
            return None;
        }
        let mut arrays: Vec<HeapArray> = self.heap.values().copied().collect();
        arrays.sort_by_key(|array| array.binding);
        Some(HeapUse { form: HeapForm::FixedArray, set: HEAP_SET, arrays })
    }
}

/// A literal string at the start of `words`, and the words it takes.
fn string(words: &[u32]) -> (String, usize) {
    let mut bytes = Vec::new();
    for (used, word) in words.iter().enumerate() {
        for byte in word.to_le_bytes() {
            if byte == 0 {
                return (String::from_utf8_lossy(&bytes).into_owned(), used + 1);
            }
            bytes.push(byte);
        }
    }
    (String::from_utf8_lossy(&bytes).into_owned(), words.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A module of a header and `instructions`, each its opcode and operand words.
    fn module(instructions: &[(u16, &[u32])]) -> Vec<u8> {
        let mut words = vec![MAGIC, 0x0001_0500, 0, 100, 0];
        for (opcode, operands) in instructions {
            let count = u32::try_from(operands.len() + 1).unwrap();
            words.push(count << 16 | u32::from(*opcode));
            words.extend_from_slice(operands);
        }
        words.iter().flat_map(|word| word.to_le_bytes()).collect()
    }

    /// `name` as a SPIR-V literal string: UTF-8, nul-terminated, padded to whole words.
    fn literal(name: &str) -> Vec<u32> {
        let mut bytes = name.as_bytes().to_vec();
        bytes.resize((bytes.len() / 4 + 1) * 4, 0);
        bytes.as_chunks::<4>().0.iter().map(|word| u32::from_le_bytes(*word)).collect()
    }

    #[test]
    fn bytes_that_are_not_a_module_are_refused() {
        let capability = module(&[(OP_CAPABILITY, &[1])]);
        let mut past = capability.clone();
        past[22] = 3; // The capability's word count, the high half of its first word, made 3.
        for (bytes, says) in [
            (&b""[..], "not a SPIR-V module"),
            (&[0; 20], "not a little-endian SPIR-V module"),
            (&capability[..23], "not a SPIR-V module"),
            (&past, "an instruction at word 5 runs past the module"),
            (&module(&[(OP_ENTRY_POINT, &[0])]), "opcode 15 has too few operands"),
        ] {
            let error = reflect(bytes).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
            assert!(error.message().starts_with(says), "{error}");
        }
    }

    #[test]
    fn a_capability_or_extension_off_the_list_is_refused() {
        let shader = (OP_CAPABILITY, &[1][..]);
        let error = reflect(&module(&[shader, (OP_CAPABILITY, &[12])])).unwrap_err();
        assert!(error.message().contains("capability Int64Atomics"), "{error}");
        let error = reflect(&module(&[shader, (OP_CAPABILITY, &[6])])).unwrap_err();
        assert!(error.message().contains("capability 6, which is outside"), "{error}");
        let extension = literal("SPV_EXT_descriptor_indexing");
        let error = reflect(&module(&[shader, (OP_EXTENSION, &extension)])).unwrap_err();
        assert!(error.message().contains("extension SPV_EXT_descriptor_indexing"), "{error}");
    }

    #[test]
    fn a_module_of_two_entry_points_is_refused_and_both_are_read() {
        let mut vertex = vec![0, 1];
        vertex.extend(literal("vs_main"));
        let mut fragment = vec![4, 2];
        fragment.extend(literal("fs_main"));
        let code = module(&[
            (OP_CAPABILITY, &[1]),
            (OP_ENTRY_POINT, &vertex),
            (OP_ENTRY_POINT, &fragment),
        ]);
        assert_eq!(entry_points(&code).unwrap(), ["vs_main", "fs_main"]);
        let error = reflect(&code).unwrap_err();
        assert!(error.message().starts_with("2 entry points, where one is cooked"), "{error}");
    }
}
