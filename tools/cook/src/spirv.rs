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
    ComponentType, ConstantSlot, ConstantType, HEAP_SET, HeapArray, HeapArrayKind, HeapForm,
    HeapUse, PUSH_BYTES_MAX, ShaderStage, VARYING_LOCATIONS, Varying, WORKGROUP_INVOCATIONS_MAX,
    WORKGROUP_SIZE_MAX,
};

const MAGIC: u32 = 0x0723_0203;

// Opcodes.
const OP_EXTENSION: u16 = 10;
const OP_ENTRY_POINT: u16 = 15;
const OP_EXECUTION_MODE: u16 = 16;
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
const OP_SAMPLED_IMAGE: u16 = 86;
const OP_CONVERT_PTR_TO_U: u16 = 117;
const OP_BITCAST: u16 = 124;
const OP_SELECT: u16 = 169;
const OP_PHI: u16 = 245;
const OP_EXECUTION_MODE_ID: u16 = 331;

// Execution modes.
const LOCAL_SIZE: u32 = 17;
const LOCAL_SIZE_ID: u32 = 38;

// Decorations.
const SPEC_ID: u32 = 1;
const ROW_MAJOR: u32 = 4;
const ARRAY_STRIDE: u32 = 6;
const MATRIX_STRIDE: u32 = 7;
const BUILT_IN: u32 = 11;
const CENTROID: u32 = 16;
const SAMPLE: u32 = 17;
const LOCATION: u32 = 30;
const COMPONENT: u32 = 31;
const BINDING: u32 = 33;
const DESCRIPTOR_SET: u32 = 34;
const OFFSET: u32 = 35;
const INPUT_ATTACHMENT_INDEX: u32 = 43;
const NON_UNIFORM: u32 = 5300;

// Storage classes.
const UNIFORM_CONSTANT: u32 = 0;
const INPUT: u32 = 1;
const UNIFORM: u32 = 2;
const OUTPUT: u32 = 3;
const PUSH_CONSTANT: u32 = 9;
const STORAGE_BUFFER: u32 = 12;
const PHYSICAL_STORAGE_BUFFER: u32 = 5349;

/// `BuiltIn WorkgroupSize`.
const WORKGROUP_SIZE: u32 = 25;

/// The components a stage's varyings and the built-ins of its interface take at most:
/// Vulkan's least `maxVertexOutputComponents` and `maxFragmentInputComponents`.
const VARYING_COMPONENTS_MAX: u32 = 64;

// Image dimensions.
const DIM_2D: u32 = 1;
const DIM_SUBPASS_DATA: u32 = 6;

/// The capabilities `rhi-api`'s README lists, each of which the required set enables, by
/// number and name.
const ALLOWED: [(u32, &str); 8] = [
    (1, "Shader"),
    (11, "Int64"),
    (40, "InputAttachment"),
    (50, "ImageQuery"),
    (55, "StorageImageReadWithoutFormat"),
    (5301, "ShaderNonUniform"),
    (5307, "SampledImageArrayNonUniformIndexing"),
    (5347, "PhysicalStorageBufferAddresses"),
];

/// The extension the list's `PhysicalStorageBufferAddresses` needs, and the only one.
const EXTENSION: &str = "SPV_KHR_physical_storage_buffer";

/// What a refused capability most often means, for those the README names.
const REFUSED: [(u32, &str, &str); 7] = [
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
    (
        9,
        "Float16",
        "a `half` declares it, and the format's constants and varyings are of 32 bits: use \
         `float`",
    ),
    (
        22,
        "Int16",
        "a 16-bit integer declares it, and the format's constants and varyings are of 32 \
         bits: use `int` or `uint`",
    ),
    (
        10,
        "Float64",
        "a `double` declares it, and the format's constants and varyings are of 32 bits: use \
         `float`",
    ),
    (
        35,
        "SampleRateShading",
        "`sample` interpolation declares it, which the cook refuses until an attachment can \
         be multisampled",
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
    /// A vertex entry's outputs, a fragment entry's inputs, by location.
    pub varyings: Vec<Varying>,
    pub outputs: u32,
    /// A compute entry's `LocalSize`.
    pub workgroup: Option<[u32; 3]>,
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
    let (varyings, outputs) = match stage {
        ShaderStage::Vertex => {
            module.check_no_inputs(entry, "vertex")?;
            (module.varyings(entry, OUTPUT)?, 0)
        }
        ShaderStage::Fragment => {
            (module.varyings(entry, INPUT)?, module.locations(entry, OUTPUT, "fragment output")?)
        }
        ShaderStage::Compute => {
            module.check_no_inputs(entry, "compute")?;
            (Vec::new(), 0)
        }
    };
    let workgroup = module.workgroup(stage)?;
    Ok(Reflection {
        stage,
        entry: entry.name.clone(),
        heap: descriptors.heap_use(),
        constants: module.constants()?,
        push_bytes: module.push_bytes()?,
        inputs: descriptors.inputs,
        varyings,
        outputs,
        workgroup,
    })
}

fn invalid(message: impl Into<std::borrow::Cow<'static, str>>) -> Error {
    Error::new(ErrorKind::InvalidData, message)
}

#[derive(Clone, Debug)]
enum Type {
    Bool,
    Int { width: u32, signed: bool },
    Float { width: u32 },
    Vector { component: u32, count: u32 },
    Matrix { column: u32, count: u32 },
    Image(Image),
    Sampler,
    Array { element: u32, length: u32 },
    RuntimeArray,
    Struct { members: Vec<u32> },
    Pointer { storage: u32, pointee: u32 },
    Other,
}

/// An `OpTypeImage`'s operands.
#[derive(Clone, Copy, Debug)]
struct Image {
    /// The type of its texels' components.
    sampled_type: u32,
    dim: u32,
    depth: u32,
    arrayed: u32,
    multisampled: u32,
    /// 1 for an image read through a sampler, 2 for a storage image, 0 for one known only at
    /// run time.
    sampled: u32,
    format: u32,
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
    /// Each `OpExecutionMode` and `OpExecutionModeId`: the mode and its operands.
    modes: Vec<(u32, Vec<u32>)>,
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
            // The one entry point a cooked module has is the one each mode is of.
            OP_EXECUTION_MODE | OP_EXECUTION_MODE_ID => {
                let operands = operands.get(2..).unwrap_or_default().to_vec();
                self.modes.push((word(1)?, operands));
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
            OP_TYPE_INT => {
                let int = Type::Int { width: word(1)?, signed: word(2)? != 0 };
                self.types.insert(word(0)?, int);
            }
            OP_TYPE_FLOAT => _ = self.types.insert(word(0)?, Type::Float { width: word(1)? }),
            OP_TYPE_VECTOR => {
                let vector = Type::Vector { component: word(1)?, count: word(2)? };
                self.types.insert(word(0)?, vector);
            }
            OP_TYPE_MATRIX => {
                let matrix = Type::Matrix { column: word(1)?, count: word(2)? };
                self.types.insert(word(0)?, matrix);
            }
            OP_TYPE_IMAGE => {
                let image = Image {
                    sampled_type: word(1)?,
                    dim: word(2)?,
                    depth: word(3)?,
                    arrayed: word(4)?,
                    multisampled: word(5)?,
                    sampled: word(6)?,
                    format: word(7)?,
                };
                self.types.insert(word(0)?, Type::Image(image));
            }
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

    fn member_is(&self, member: (u32, u32), decoration: u32) -> bool {
        self.member_decorations
            .get(&member)
            .is_some_and(|all| all.iter().any(|(d, _)| *d == decoration))
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
                        (0, Some(Type::Image(image))) => {
                            if let Some(fault) = self.heap_image_fault(image) {
                                return Err(invalid(format!(
                                    "set {HEAP_SET}, binding 0 is the heap's array of 2D images \
                                     of 32-bit floats, each read through a sampler, and the \
                                     module's image there {fault}: the heap's images are \
                                     `lactose.heap`'s `Texture2D<float4>`"
                                )));
                            }
                            HeapArrayKind::SampledImage2d
                        }
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
                (0, _, Some(Type::Image(Image { dim: DIM_SUBPASS_DATA, .. }))) => {
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

    /// What makes `image` other than the heap's element, which `HeapArrayKind::SampledImage2d`
    /// names and every view the heap holds is: `None` if nothing does. A depth of 0 or 2 is
    /// the same image; Slang writes 2 for `Texture2D<float>`.
    fn heap_image_fault(&self, image: &Image) -> Option<&'static str> {
        [
            (
                !matches!(self.types.get(&image.sampled_type), Some(Type::Float { width: 32 })),
                "has texels that are not 32-bit floats",
            ),
            (image.dim != DIM_2D, "is not 2D"),
            (!matches!(image.depth, 0 | 2), "is a depth image"),
            (image.arrayed != 0, "is arrayed"),
            (image.multisampled != 0, "is multisampled"),
            (image.sampled != 1, "is not read through a sampler"),
            (image.format != 0, "declares a texel format"),
        ]
        .into_iter()
        .find_map(|(fault, says)| fault.then_some(says))
    }

    /// Every access through the heap carries the non-uniform decoration: each access chain
    /// into one of its arrays, each load through one, and each `OpSampledImage` that combines
    /// a loaded image or sampler, the operand Vulkan's rule names for a sample. A use of the
    /// array itself, rather than of an element reached by an access chain, is refused, and so
    /// is a pointer into the heap or a value loaded through it passed on in any other way: the
    /// check would not see the access.
    fn check_heap_accesses(&self, heap: &BTreeMap<u32, HeapArray>) -> Result<()> {
        // The access chains' results, and the values loaded or combined from them.
        let mut reached: BTreeSet<u32> = BTreeSet::new();
        let mut loaded: BTreeSet<u32> = BTreeSet::new();
        for instruction in &self.body {
            let operands = &instruction.operands;
            let pointer = |id: &u32| heap.contains_key(id) || reached.contains(id);
            let through = |id: &u32| pointer(id) || loaded.contains(id);
            let undecorated = |what: &str, result: u32| {
                invalid(format!(
                    "{what} through the heap, %{result}, carries no non-uniform decoration"
                ))
            };
            match instruction.op {
                OP_ACCESS_CHAIN
                | OP_IN_BOUNDS_ACCESS_CHAIN
                | OP_PTR_ACCESS_CHAIN
                | OP_IN_BOUNDS_PTR_ACCESS_CHAIN
                    if operands.get(2).is_some_and(pointer) =>
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
                        return Err(undecorated("a load", result));
                    }
                    loaded.insert(result);
                }
                OP_SAMPLED_IMAGE if operands.iter().skip(2).any(|id| loaded.contains(id)) => {
                    let result = operands[1];
                    if !self.is(result, NON_UNIFORM) {
                        return Err(undecorated("a sampled image", result));
                    }
                    loaded.insert(result);
                }
                OP_STORE | OP_COPY_MEMORY | OP_COPY_MEMORY_SIZED
                    if operands.iter().take(2).any(through) =>
                {
                    return Err(invalid(
                        "the heap, or a value loaded through it, is written or copied through a \
                         pointer",
                    ));
                }
                OP_FUNCTION_CALL | OP_COPY_OBJECT | OP_CONVERT_PTR_TO_U | OP_BITCAST
                | OP_SELECT | OP_PHI
                    if operands.iter().skip(2).any(through) =>
                {
                    return Err(invalid(format!(
                        "a pointer into the heap or a value loaded through it is passed on \
                         (opcode {}), where the check follows an access chain, a load and a \
                         sampled image alone",
                        instruction.op
                    )));
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Bit `i` for each variable of the entry's interface in `storage` at location `i`, the
    /// built-ins aside: each has one location and takes one, with no `Component` of its own.
    fn locations(&self, entry: &Entry, storage: u32, what: &str) -> Result<u32> {
        let mut bits = 0;
        for &variable in &entry.interface {
            let Some(&(_, class)) = self.variables.get(&variable) else { continue };
            if class != storage || self.is_built_in(variable)? {
                continue;
            }
            let (pointee, _) = self.pointee(variable)?;
            let locations: Vec<u32> =
                self.decorations(variable, LOCATION).filter_map(|o| o.first().copied()).collect();
            let [location] = locations.as_slice() else {
                return Err(invalid(format!(
                    "a {what}, %{variable}, with {} locations, where each has one",
                    locations.len()
                )));
            };
            if self.is(variable, COMPONENT) {
                return Err(invalid(format!(
                    "the {what} at location {location} has a `Component` decoration, which \
                     places it beside another in one location, where each takes a location of \
                     its own"
                )));
            }
            if matches!(
                self.types.get(&pointee),
                Some(Type::Array { .. } | Type::Struct { .. } | Type::Matrix { .. })
            ) {
                return Err(invalid(format!(
                    "the {what} at location {location} is an array, a matrix or a struct, which \
                     takes more than one location"
                )));
            }
            if *location >= u32::BITS || bits & (1 << location) != 0 {
                return Err(invalid(format!("{what} location {location} twice or out of range")));
            }
            bits |= 1 << location;
        }
        Ok(bits)
    }

    /// The varyings of `storage` the entry point declares, by location: each a scalar or a
    /// vector of 32-bit floats or integers, at a location below `VARYING_LOCATIONS`, neither
    /// `centroid` nor `sample`. With the built-ins of the interface they take at most
    /// `VARYING_COMPONENTS_MAX` components, a location's four up to the highest.
    fn varyings(&self, entry: &Entry, storage: u32) -> Result<Vec<Varying>> {
        self.locations(entry, storage, "varying")?;
        let mut varyings = Vec::new();
        for &variable in &entry.interface {
            let Some(&(_, class)) = self.variables.get(&variable) else { continue };
            if class != storage || self.is_built_in(variable)? {
                continue;
            }
            let (pointee, _) = self.pointee(variable)?;
            let location = self.decoration(variable, LOCATION).expect("held to one by locations");
            let (component, count) = match self.types.get(&pointee) {
                Some(Type::Vector { component, count }) => (*component, *count),
                _ => (pointee, 1),
            };
            let component = match self.types.get(&component) {
                Some(Type::Float { width: 32 }) => Some(ComponentType::Float),
                Some(Type::Int { width: 32, signed: true }) => Some(ComponentType::Int),
                Some(Type::Int { width: 32, signed: false }) => Some(ComponentType::Uint),
                _ => None,
            };
            let Some(component) = component.filter(|_| (1..=4).contains(&count)) else {
                return Err(invalid(format!(
                    "the varying at location {location} is {}, where a varying is a float, an \
                     int or a uint of 32 bits, or a vector of two to four of them",
                    type_name(self.types.get(&pointee))
                )));
            };
            if location >= VARYING_LOCATIONS {
                return Err(invalid(format!(
                    "the varying at location {location}, where a varying is below location \
                     {VARYING_LOCATIONS}, the format's bound"
                )));
            }
            // Slang writes `centroid` as Metal's `[[center_perspective]]`, so under
            // multisampling the two targets would sample at different points.
            for (decoration, name) in [(CENTROID, "centroid"), (SAMPLE, "sample")] {
                if self.is(variable, decoration) {
                    return Err(invalid(format!(
                        "the varying at location {location} is `{name}`, which the cook refuses \
                         until an attachment can be multisampled"
                    )));
                }
            }
            varyings.push(Varying { location, component, count });
        }
        varyings.sort_by_key(|varying| varying.location);
        let locations = varyings.last().map_or(0, |highest| 4 * (highest.location + 1));
        let mut built_ins = 0;
        for &variable in &entry.interface {
            let Some(&(_, class)) = self.variables.get(&variable) else { continue };
            if class == storage && self.is_built_in(variable)? {
                built_ins += self.components(self.pointee(variable)?.0)?;
            }
        }
        if locations + built_ins > VARYING_COMPONENTS_MAX {
            let what = if storage == OUTPUT {
                "vertex entry's outputs"
            } else {
                "fragment entry's inputs"
            };
            return Err(invalid(format!(
                "the {what} take {} components, {locations} for the locations up to the highest \
                 and {built_ins} for the built-ins, above the {VARYING_COMPONENTS_MAX} of \
                 Vulkan's least `maxVertexOutputComponents` and `maxFragmentInputComponents`, \
                 which count both (VUID-RuntimeSpirv-Location-06272)",
                locations + built_ins
            )));
        }
        Ok(varyings)
    }

    /// The 32-bit components a value of type `ty` takes in an interface: a 64-bit scalar's
    /// two, each element's and each member's.
    fn components(&self, ty: u32) -> Result<u32> {
        let components = match self.types.get(&ty) {
            Some(Type::Bool) => 1,
            Some(Type::Int { width, .. } | Type::Float { width }) => width.div_ceil(32),
            Some(Type::Vector { component, count }) => self.components(*component)? * count,
            Some(Type::Matrix { column, count }) => self.components(*column)? * count,
            Some(Type::Array { element, length }) if self.constants.contains_key(length) => {
                self.components(*element)? * self.constants[length]
            }
            Some(Type::Struct { members }) => {
                members.iter().map(|&member| self.components(member)).sum::<Result<u32>>()?
            }
            _ => {
                return Err(invalid(format!(
                    "a built-in of type %{ty} in the interface, whose components the cook cannot \
                     count"
                )));
            }
        };
        Ok(components)
    }

    /// A compute entry's workgroup, from its `LocalSize`, within the format's bounds; `None`
    /// for another stage.
    fn workgroup(&self, stage: ShaderStage) -> Result<Option<[u32; 3]>> {
        let overridden = self.decorations.values().flatten().any(|(decoration, operands)| {
            *decoration == BUILT_IN && operands.first() == Some(&WORKGROUP_SIZE)
        });
        if overridden {
            return Err(invalid(
                "an id decorated `BuiltIn WorkgroupSize`, which sets the workgroup in place of \
                 `LocalSize`, from a constant a specialisation may change, and which the cooked \
                 format does not hold: give `[numthreads]` literal sizes",
            ));
        }
        if self.modes.iter().any(|(mode, _)| *mode == LOCAL_SIZE_ID) {
            return Err(invalid(
                "the entry's workgroup size is `LocalSizeId`, of ids a specialisation may \
                 change, which a Vulkan device takes under `maintenance4` alone \
                 (VUID-RuntimeSpirv-LocalSizeId-06434) and the cooked format does not hold: \
                 give `[numthreads]` literal sizes",
            ));
        }
        let sizes: Vec<&[u32]> = self
            .modes
            .iter()
            .filter(|(mode, _)| *mode == LOCAL_SIZE)
            .map(|(_, operands)| operands.as_slice())
            .collect();
        match (stage, sizes.as_slice()) {
            (ShaderStage::Compute, [[x, y, z]]) => {
                let size = [*x, *y, *z];
                let within =
                    size.iter().zip(WORKGROUP_SIZE_MAX).all(|(&n, most)| (1..=most).contains(&n));
                if !within || x * y * z > WORKGROUP_INVOCATIONS_MAX {
                    return Err(invalid(format!(
                        "the entry's workgroup is {size:?}, where each dimension is 1 to \
                         {WORKGROUP_SIZE_MAX:?} and their product at most \
                         {WORKGROUP_INVOCATIONS_MAX}: Vulkan's least limits"
                    )));
                }
                Ok(Some(size))
            }
            (ShaderStage::Compute, _) => Err(invalid(format!(
                "a compute entry with {} `LocalSize` modes of three sizes, where it has one",
                sizes.len()
            ))),
            (_, []) => Ok(None),
            (_, _) => Err(invalid("a `LocalSize` outside a compute entry")),
        }
    }

    /// A vertex or compute entry reads no input but the built-ins: a pipeline has no vertex
    /// buffer binding, and a compute entry has no stage before it.
    fn check_no_inputs(&self, entry: &Entry, stage: &str) -> Result<()> {
        for &variable in &entry.interface {
            let Some(&(_, INPUT)) = self.variables.get(&variable) else { continue };
            if !self.is_built_in(variable)? {
                let at = match self.decoration(variable, LOCATION) {
                    Some(location) => format!("at location {location}"),
                    None => "with no location".to_owned(),
                };
                return Err(invalid(format!(
                    "a {stage} entry's input {at}, %{variable}, that is not a built-in: a \
                     pipeline has no vertex buffer binding, so vertices are read through a \
                     device address, as `shaders/surface.slang` reads them"
                )));
            }
        }
        Ok(())
    }

    /// Whether a stage variable is a built-in: decorated as one, or a block whose members are.
    fn is_built_in(&self, variable: u32) -> Result<bool> {
        if self.is(variable, BUILT_IN) {
            return Ok(true);
        }
        let (pointee, _) = self.pointee(variable)?;
        Ok(matches!(self.types.get(&pointee), Some(Type::Struct { .. }))
            && self.member_decorations.iter().any(|(&(ty, _), all)| {
                ty == pointee && all.iter().any(|(decoration, _)| *decoration == BUILT_IN)
            }))
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
            let ty = match self.types.get(&ty) {
                Some(Type::Bool) => ConstantType::Bool,
                Some(Type::Int { width: 32, signed: true }) => ConstantType::Int,
                Some(Type::Int { width: 32, signed: false }) => ConstantType::Uint,
                Some(Type::Float { width: 32 }) => ConstantType::Float,
                other => {
                    return Err(invalid(format!(
                        "specialisation constant {slot} is {}, where the format holds a bool, \
                         an int, a uint or a float, the last three of 32 bits",
                        type_name(other)
                    )));
                }
            };
            constants.push(ConstantSlot { id: slot, ty });
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
    /// or the member that holds the array it is an element of, for a matrix's stride and
    /// order. A 3-component vector is refused at any depth: SPIR-V's layout gives it 12 bytes
    /// and the MSL's struct 16, so a member after one is at another offset on each target. A
    /// matrix's columns or rows of 3 are not: its `MatrixStride` is theirs on both targets.
    fn size(&self, ty: u32, member: Option<(u32, u32)>) -> Result<u32> {
        let unsized_ = || {
            invalid(format!("the push-constant block holds type %{ty}, which the cook cannot size"))
        };
        let three = || {
            invalid(
                "the push-constant block holds a 3-component vector, which the SPIR-V lays out in \
                 12 bytes and the MSL in 16: use 4 components",
            )
        };
        let size = match self.types.get(&ty).ok_or_else(unsized_)? {
            Type::Int { width, .. } | Type::Float { width } => width / 8,
            Type::Vector { count: 3, .. } => return Err(three()),
            Type::Vector { component, count } => self.size(*component, None)? * count,
            Type::Matrix { column, count } => {
                let Some(Type::Vector { count: rows, .. }) = self.types.get(column) else {
                    return Err(unsized_());
                };
                let stride = member.and_then(|m| self.member_decoration(m, MATRIX_STRIDE));
                let stride = stride.ok_or_else(unsized_)?;
                // A row-major matrix is a stride per row, a column-major one per column.
                let row_major = member.is_some_and(|m| self.member_is(m, ROW_MAJOR));
                stride * if row_major { rows } else { count }
            }
            Type::Array { element, length } => {
                // The element's own size is the stride's; it is read for its members alone.
                self.size(*element, member)?;
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

/// A type's name in a message.
fn type_name(ty: Option<&Type>) -> String {
    match ty {
        Some(Type::Bool) => "a bool".to_owned(),
        Some(Type::Int { width, signed: true }) => format!("an int of {width} bits"),
        Some(Type::Int { width, signed: false }) => format!("a uint of {width} bits"),
        Some(Type::Float { width }) => format!("a float of {width} bits"),
        Some(Type::Vector { count, .. }) => format!("a vector of {count} of another type"),
        _ => "of another type".to_owned(),
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
    fn each_capability_on_the_list_is_read() {
        for (capability, name) in ALLOWED {
            let declared = [(OP_CAPABILITY, vec![capability])];
            let read = reflect(&assemble(FRAGMENT, &[], &declared, &[]));
            assert!(read.is_ok(), "{name}: {:?}", read.err());
        }
        // `ImageQuery`, which a size query of a heap image declares.
        reflect(&assemble(FRAGMENT, &[], &[(OP_CAPABILITY, vec![50])], &[])).unwrap();
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

    #[test]
    fn the_refused_capabilities_say_why() {
        let shader = (OP_CAPABILITY, &[1][..]);
        for (capability, says) in [
            (4427, "capability DrawParameters, which is outside rhi-api's list: `SV_VertexID`"),
            (5302, "capability RuntimeDescriptorArray, which is outside rhi-api's list: a runtime"),
            (12, "capability Int64Atomics, which is outside rhi-api's list: 64-bit atomics"),
            (9, "capability Float16, which is outside rhi-api's list: a `half` declares it"),
            (22, "capability Int16, which is outside rhi-api's list: a 16-bit integer"),
            (10, "capability Float64, which is outside rhi-api's list: a `double` declares it"),
            (35, "capability SampleRateShading, which is outside rhi-api's list: `sample`"),
        ] {
            let error = reflect(&module(&[shader, (OP_CAPABILITY, &[capability])])).unwrap_err();
            assert!(error.message().starts_with(&format!("the module declares {says}")), "{error}");
        }
    }

    // The ids of the modules below.
    const MAIN: u32 = 1;
    const VOID: u32 = 2;
    const FLOAT: u32 = 3;
    const UINT: u32 = 4;
    const VEC4: u32 = 5;
    const VEC3: u32 = 6;
    const N1024: u32 = 7;
    const N0: u32 = 8;
    const IMAGE: u32 = 9;
    const SAMPLER: u32 = 10;
    const SAMPLED_IMAGE: u32 = 11;
    const IMAGES: u32 = 12;
    const SAMPLERS: u32 = 13;
    const IMAGES_POINTER: u32 = 14;
    const SAMPLERS_POINTER: u32 = 15;
    const IMAGE_POINTER: u32 = 16;
    const SAMPLER_POINTER: u32 = 17;
    const HEAP_IMAGES: u32 = 18;
    const HEAP_SAMPLERS: u32 = 19;
    /// The heap's sample, as Slang writes it: an access chain into each array, a load
    /// through each, and the sampled image of the two.
    const IMAGE_AT: u32 = 20;
    const IMAGE_LOADED: u32 = 21;
    const SAMPLER_AT: u32 = 22;
    const SAMPLER_LOADED: u32 = 23;
    const COMBINED: u32 = 24;
    /// The first id each test may use.
    const FREE: u32 = 40;

    const VERTEX: u32 = 0;
    const FRAGMENT: u32 = 4;
    const COMPUTE: u32 = 5;

    type Instructions = Vec<(u16, Vec<u32>)>;

    /// A module of one entry point, `main`, of `model` and with `interface`, a compute entry's
    /// workgroup of one invocation: the types every test shares, then `globals`, then one
    /// function of `body`. A later type of an id replaces an earlier one.
    fn assemble(
        model: u32,
        interface: &[u32],
        globals: &[(u16, Vec<u32>)],
        body: &[(u16, Vec<u32>)],
    ) -> Vec<u8> {
        let mut entry = vec![model, MAIN];
        entry.extend(literal("main"));
        entry.extend_from_slice(interface);
        let mut all: Instructions = vec![(OP_CAPABILITY, vec![1]), (OP_ENTRY_POINT, entry)];
        if model == COMPUTE {
            all.push((OP_EXECUTION_MODE, vec![MAIN, LOCAL_SIZE, 1, 1, 1]));
        }
        all.extend([
            (19, vec![VOID]),
            (OP_TYPE_FLOAT, vec![FLOAT, 32]),
            (OP_TYPE_INT, vec![UINT, 32, 0]),
            (OP_TYPE_VECTOR, vec![VEC4, FLOAT, 4]),
            (OP_TYPE_VECTOR, vec![VEC3, FLOAT, 3]),
            (OP_CONSTANT, vec![UINT, N1024, 1024]),
            (OP_CONSTANT, vec![UINT, N0, 0]),
            (OP_TYPE_IMAGE, vec![IMAGE, FLOAT, DIM_2D, 2, 0, 0, 1, 0]),
            (OP_TYPE_SAMPLER, vec![SAMPLER]),
            (27, vec![SAMPLED_IMAGE, IMAGE]),
            (OP_TYPE_POINTER, vec![IMAGE_POINTER, UNIFORM_CONSTANT, IMAGE]),
            (OP_TYPE_POINTER, vec![SAMPLER_POINTER, UNIFORM_CONSTANT, SAMPLER]),
        ]);
        all.extend_from_slice(globals);
        all.push((54, vec![VOID, MAIN, 0, FREE + 100]));
        all.extend_from_slice(body);
        all.push((56, vec![]));
        let instructions: Vec<(u16, &[u32])> =
            all.iter().map(|(opcode, operands)| (*opcode, operands.as_slice())).collect();
        module(&instructions)
    }

    /// The heap's two arrays, of `length` each.
    fn heap(length: u32) -> Instructions {
        vec![
            (OP_TYPE_ARRAY, vec![IMAGES, IMAGE, length]),
            (OP_TYPE_ARRAY, vec![SAMPLERS, SAMPLER, length]),
            (OP_TYPE_POINTER, vec![IMAGES_POINTER, UNIFORM_CONSTANT, IMAGES]),
            (OP_TYPE_POINTER, vec![SAMPLERS_POINTER, UNIFORM_CONSTANT, SAMPLERS]),
            (OP_VARIABLE, vec![IMAGES_POINTER, HEAP_IMAGES, UNIFORM_CONSTANT]),
            (OP_VARIABLE, vec![SAMPLERS_POINTER, HEAP_SAMPLERS, UNIFORM_CONSTANT]),
            (OP_DECORATE, vec![HEAP_IMAGES, DESCRIPTOR_SET, HEAP_SET]),
            (OP_DECORATE, vec![HEAP_IMAGES, BINDING, 0]),
            (OP_DECORATE, vec![HEAP_SAMPLERS, DESCRIPTOR_SET, HEAP_SET]),
            (OP_DECORATE, vec![HEAP_SAMPLERS, BINDING, 1]),
        ]
    }

    /// The heap's sample, and `after` it.
    fn sample(after: &[(u16, Vec<u32>)]) -> Instructions {
        let mut body = vec![
            (OP_ACCESS_CHAIN, vec![IMAGE_POINTER, IMAGE_AT, HEAP_IMAGES, N0]),
            (OP_LOAD, vec![IMAGE, IMAGE_LOADED, IMAGE_AT]),
            (OP_ACCESS_CHAIN, vec![SAMPLER_POINTER, SAMPLER_AT, HEAP_SAMPLERS, N0]),
            (OP_LOAD, vec![SAMPLER, SAMPLER_LOADED, SAMPLER_AT]),
            (OP_SAMPLED_IMAGE, vec![SAMPLED_IMAGE, COMBINED, IMAGE_LOADED, SAMPLER_LOADED]),
        ];
        body.extend_from_slice(after);
        body
    }

    /// A fragment module that samples the heap, with each of `decorated` non-uniform, then
    /// does `after`, with `globals` beside the heap.
    fn heap_module(
        decorated: &[u32],
        globals: &[(u16, Vec<u32>)],
        after: &[(u16, Vec<u32>)],
    ) -> Vec<u8> {
        let mut all = heap(N1024);
        all.extend(decorated.iter().map(|&id| (OP_DECORATE, vec![id, NON_UNIFORM])));
        all.extend_from_slice(globals);
        assemble(FRAGMENT, &[], &all, &sample(after))
    }

    const DECORATED: [u32; 5] = [IMAGE_AT, IMAGE_LOADED, SAMPLER_AT, SAMPLER_LOADED, COMBINED];

    fn refused(code: &[u8], says: &str) {
        let error = reflect(code).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
        assert!(error.message().contains(says), "{says}: {error}");
    }

    #[test]
    fn the_heap_sampled_with_every_decoration_is_read() {
        let reflection = reflect(&heap_module(&DECORATED, &[], &[])).unwrap();
        let arrays = vec![
            HeapArray { kind: HeapArrayKind::SampledImage2d, binding: 0, capacity: 1024 },
            HeapArray { kind: HeapArrayKind::Sampler, binding: 1, capacity: 1024 },
        ];
        assert_eq!(
            reflection.heap,
            Some(HeapUse { form: HeapForm::FixedArray, set: HEAP_SET, arrays })
        );
        assert_eq!(reflection.stage, ShaderStage::Fragment);
    }

    #[test]
    fn each_step_of_a_sample_without_the_decoration_is_refused() {
        for (bare, says) in [
            (IMAGE_AT, "an access through the heap, %20, carries no non-uniform decoration"),
            (IMAGE_LOADED, "a load through the heap, %21, carries no non-uniform decoration"),
            (SAMPLER_AT, "an access through the heap, %22, carries no non-uniform decoration"),
            (SAMPLER_LOADED, "a load through the heap, %23, carries no non-uniform decoration"),
            (COMBINED, "a sampled image through the heap, %24, carries no non-uniform decoration"),
        ] {
            let decorated: Vec<u32> = DECORATED.into_iter().filter(|&id| id != bare).collect();
            refused(&heap_module(&decorated, &[], &[]), says);
        }
        // An access chain through another one is an access too.
        let deeper = [(OP_ACCESS_CHAIN, vec![IMAGE_POINTER, FREE, IMAGE_AT])];
        refused(&heap_module(&DECORATED, &[], &deeper), "an access through the heap, %40");
        // Each opcode of an access chain, the pointer forms included.
        for op in [OP_IN_BOUNDS_ACCESS_CHAIN, OP_PTR_ACCESS_CHAIN, OP_IN_BOUNDS_PTR_ACCESS_CHAIN] {
            let other = [(op, vec![IMAGE_POINTER, FREE, IMAGE_AT, N0])];
            refused(&heap_module(&DECORATED, &[], &other), "an access through the heap, %40");
        }
    }

    #[test]
    fn the_heap_used_whole_written_or_passed_on_is_refused() {
        let whole = [(OP_LOAD, vec![IMAGES, FREE, HEAP_IMAGES])];
        refused(&heap_module(&DECORATED, &[], &whole), "the heap's array is loaded whole");
        for written in [
            (OP_STORE, vec![IMAGE_AT, FREE]),
            (OP_STORE, vec![FREE, IMAGE_LOADED]),
            (OP_COPY_MEMORY, vec![FREE, SAMPLER_AT]),
            (OP_COPY_MEMORY_SIZED, vec![HEAP_SAMPLERS, FREE, N0]),
        ] {
            let says = "the heap, or a value loaded through it, is written or copied through a \
                        pointer";
            refused(&heap_module(&DECORATED, &[], &[written]), says);
        }
        for (op, operands) in [
            (OP_FUNCTION_CALL, vec![VOID, FREE, FREE + 1, IMAGE_AT]),
            (OP_FUNCTION_CALL, vec![VOID, FREE, FREE + 1, COMBINED]),
            (OP_COPY_OBJECT, vec![IMAGE, FREE, IMAGE_LOADED]),
            (OP_CONVERT_PTR_TO_U, vec![UINT, FREE, SAMPLER_AT]),
            (OP_BITCAST, vec![UINT, FREE, HEAP_IMAGES]),
            (OP_SELECT, vec![SAMPLER, FREE, FREE + 1, SAMPLER_LOADED, SAMPLER_LOADED]),
            (OP_PHI, vec![IMAGE_POINTER, FREE, IMAGE_AT, FREE + 1]),
        ] {
            let says = format!("is passed on (opcode {op})");
            refused(&heap_module(&DECORATED, &[], &[(op, operands)]), &says);
        }
    }

    #[test]
    fn a_heap_image_of_another_form_is_refused() {
        let int = (OP_TYPE_INT, vec![FREE, 32, 0]);
        for (image, says) in [
            (vec![IMAGE, FREE, DIM_2D, 2, 0, 0, 1, 0], "has texels that are not 32-bit floats"),
            (vec![IMAGE, FLOAT, 2, 2, 0, 0, 1, 0], "is not 2D"),
            (vec![IMAGE, FLOAT, DIM_2D, 1, 0, 0, 1, 0], "is a depth image"),
            (vec![IMAGE, FLOAT, DIM_2D, 2, 1, 0, 1, 0], "is arrayed"),
            (vec![IMAGE, FLOAT, DIM_2D, 2, 0, 1, 1, 0], "is multisampled"),
            (vec![IMAGE, FLOAT, DIM_2D, 2, 0, 0, 2, 0], "is not read through a sampler"),
            (vec![IMAGE, FLOAT, DIM_2D, 2, 0, 0, 1, 2], "declares a texel format"),
        ] {
            let globals = [int.clone(), (OP_TYPE_IMAGE, image)];
            let says = format!(
                "binding 0 is the heap's array of 2D images of 32-bit floats, each read through \
                 a sampler, and the module's image there {says}"
            );
            refused(&heap_module(&DECORATED, &globals, &[]), &says);
        }
        // A depth of 0 is the same image as 2.
        let zero = [(OP_TYPE_IMAGE, vec![IMAGE, FLOAT, DIM_2D, 0, 0, 0, 1, 0])];
        reflect(&heap_module(&DECORATED, &zero, &[])).unwrap();
    }

    #[test]
    fn a_descriptor_off_the_layout_is_refused() {
        let at = |set: u32, binding: u32, ty: u32| -> Instructions {
            vec![
                (OP_TYPE_POINTER, vec![FREE, UNIFORM_CONSTANT, ty]),
                (OP_VARIABLE, vec![FREE, FREE + 1, UNIFORM_CONSTANT]),
                (OP_DECORATE, vec![FREE + 1, DESCRIPTOR_SET, set]),
                (OP_DECORATE, vec![FREE + 1, BINDING, binding]),
            ]
        };
        let subpass = (OP_TYPE_IMAGE, vec![FREE + 2, FLOAT, DIM_SUBPASS_DATA, 0, 0, 0, 2, 0]);
        let mut runtime = at(HEAP_SET, 0, FREE + 2);
        runtime.push((OP_TYPE_RUNTIME_ARRAY, vec![FREE + 2, IMAGE]));
        let mut spec_length = at(HEAP_SET, 0, FREE + 2);
        spec_length.push((OP_SPEC_CONSTANT, vec![UINT, FREE + 3, 4]));
        spec_length.push((OP_TYPE_ARRAY, vec![FREE + 2, IMAGE, FREE + 3]));
        let mut misindexed = at(0, 1, FREE + 2);
        misindexed.push(subpass.clone());
        misindexed.push((OP_DECORATE, vec![FREE + 1, INPUT_ATTACHMENT_INDEX, 0]));
        for (globals, says) in [
            (
                vec![(OP_VARIABLE, vec![IMAGE_POINTER, FREE, UNIFORM])],
                "a uniform or storage buffer, %40",
            ),
            (
                vec![(OP_VARIABLE, vec![IMAGE_POINTER, FREE, STORAGE_BUFFER])],
                "a uniform or storage buffer, %40",
            ),
            (
                vec![(OP_VARIABLE, vec![IMAGE_POINTER, FREE, UNIFORM_CONSTANT])],
                "a resource, %40, with no set and binding",
            ),
            (
                at(HEAP_SET, 2, IMAGES),
                "binding 2 there is not one of its fixed arrays (images at 0, samplers at 1)",
            ),
            (
                runtime,
                "binding 0 there is not one of its fixed arrays (images at 0, samplers at 1): it is a runtime array",
            ),
            (spec_length, "the heap's array at binding 0 has a length that is not a constant"),
            (
                at(2, 0, IMAGE),
                "a resource at set 2, binding 0, which is neither the heap's nor an input attachment",
            ),
            (misindexed, "an input attachment at binding 1 whose index is Some(0)"),
        ] {
            refused(&heap_module(&DECORATED, &globals, &[]), says);
        }
        // The samplers at the images' binding, and the images declared twice.
        let mut swapped = heap(N1024);
        swapped[7] = (OP_DECORATE, vec![HEAP_IMAGES, BINDING, 1]);
        swapped[9] = (OP_DECORATE, vec![HEAP_SAMPLERS, BINDING, 0]);
        let says = "binding 1 is the heap's samplers, and the module declares another type there";
        refused(&assemble(FRAGMENT, &[], &swapped, &[]), says);
        let mut twice = heap(N1024);
        twice.extend(at(HEAP_SET, 0, IMAGES));
        refused(
            &assemble(FRAGMENT, &[], &twice, &[]),
            "the heap's array of one kind is declared twice",
        );
        // An input attachment, in a fragment entry alone.
        let mut input = at(0, 0, FREE + 2);
        input.push(subpass);
        input.push((OP_DECORATE, vec![FREE + 1, INPUT_ATTACHMENT_INDEX, 0]));
        assert_eq!(reflect(&assemble(FRAGMENT, &[], &input, &[])).unwrap().inputs, 0b1);
        refused(
            &assemble(VERTEX, &[], &input, &[]),
            "an input attachment outside the fragment stage",
        );
    }

    /// A stage variable `FREE + 1` of `storage`, its pointee `ty`, each of `decorations` its.
    fn stage_variable(storage: u32, ty: u32, decorations: &[&[u32]]) -> Instructions {
        let mut all = vec![
            (OP_TYPE_POINTER, vec![FREE, storage, ty]),
            (OP_VARIABLE, vec![FREE, FREE + 1, storage]),
        ];
        for decoration in decorations {
            let mut operands = vec![FREE + 1];
            operands.extend_from_slice(decoration);
            all.push((OP_DECORATE, operands));
        }
        all
    }

    #[test]
    fn the_outputs_and_varyings_are_read_and_each_takes_one_location() {
        // A fragment output at location 3 and a varying at 1, beside a built-in of each.
        let mut globals = stage_variable(OUTPUT, VEC4, &[&[LOCATION, 3]]);
        globals.extend([
            (OP_TYPE_POINTER, vec![FREE + 2, INPUT, VEC4]),
            (OP_VARIABLE, vec![FREE + 2, FREE + 3, INPUT]),
            (OP_DECORATE, vec![FREE + 3, LOCATION, 1]),
            (OP_VARIABLE, vec![FREE + 2, FREE + 4, INPUT]),
            (OP_DECORATE, vec![FREE + 4, BUILT_IN, 15]),
        ]);
        let reflection =
            reflect(&assemble(FRAGMENT, &[FREE + 1, FREE + 3, FREE + 4], &globals, &[])).unwrap();
        let varying = Varying { location: 1, component: ComponentType::Float, count: 4 };
        assert_eq!((reflection.outputs, reflection.varyings), (0b1000, vec![varying]));
        let array = (OP_TYPE_ARRAY, vec![FREE + 2, VEC4, N1024]);
        for (mut globals, says) in [
            (stage_variable(OUTPUT, VEC4, &[]), "a fragment output, %41, with 0 locations"),
            (
                stage_variable(OUTPUT, VEC4, &[&[LOCATION, 0], &[LOCATION, 1]]),
                "a fragment output, %41, with 2 locations",
            ),
            (
                stage_variable(OUTPUT, FREE + 2, &[&[LOCATION, 0]]),
                "the fragment output at location 0 is an array, a matrix or a struct",
            ),
            (
                stage_variable(OUTPUT, VEC4, &[&[LOCATION, 32]]),
                "fragment output location 32 twice or out of range",
            ),
            (
                stage_variable(INPUT, FREE + 2, &[&[LOCATION, 2]]),
                "the varying at location 2 is an array, a matrix or a struct",
            ),
        ] {
            globals.push(array.clone());
            refused(&assemble(FRAGMENT, &[FREE + 1], &globals, &[]), says);
        }
        let mut twice = stage_variable(OUTPUT, VEC4, &[&[LOCATION, 0]]);
        twice.extend([
            (OP_VARIABLE, vec![FREE, FREE + 2, OUTPUT]),
            (OP_DECORATE, vec![FREE + 2, LOCATION, 0]),
        ]);
        refused(
            &assemble(FRAGMENT, &[FREE + 1, FREE + 2], &twice, &[]),
            "fragment output location 0 twice",
        );
        // A vertex entry's outputs are its varyings.
        let output = stage_variable(OUTPUT, VEC4, &[&[LOCATION, 2]]);
        let reflection = reflect(&assemble(VERTEX, &[FREE + 1], &output, &[])).unwrap();
        let varying = Varying { location: 2, ..varying };
        assert_eq!((reflection.outputs, reflection.varyings), (0, vec![varying]));
    }

    #[test]
    fn each_varying_is_read_with_its_type_and_one_the_format_does_not_hold_is_refused() {
        // Ids from `FREE + 10`: an int, a bool, a 64-bit uint, a 16-bit float, and vectors.
        let types = [
            (OP_TYPE_INT, vec![FREE + 10, 32, 1]),
            (OP_TYPE_BOOL, vec![FREE + 11]),
            (OP_TYPE_INT, vec![FREE + 12, 64, 0]),
            (OP_TYPE_FLOAT, vec![FREE + 13, 16]),
            (OP_TYPE_VECTOR, vec![FREE + 14, FREE + 10, 3]),
            (OP_TYPE_VECTOR, vec![FREE + 15, UINT, 2]),
            (OP_TYPE_VECTOR, vec![FREE + 16, FREE + 13, 2]),
        ];
        // Each varying, at locations 0 to 2 of a vertex entry, given in another order.
        let mut globals = types.to_vec();
        let mut interface = Vec::new();
        for (at, (ty, location)) in
            [(FREE + 14, 2), (UINT, 0), (FREE + 15, 1)].into_iter().enumerate()
        {
            let (pointer, variable) = (FREE + 20 + 2 * at as u32, FREE + 21 + 2 * at as u32);
            globals.extend([
                (OP_TYPE_POINTER, vec![pointer, OUTPUT, ty]),
                (OP_VARIABLE, vec![pointer, variable, OUTPUT]),
                (OP_DECORATE, vec![variable, LOCATION, location]),
            ]);
            interface.push(variable);
        }
        let reflection = reflect(&assemble(VERTEX, &interface, &globals, &[])).unwrap();
        let read: Vec<(u32, ComponentType, u32)> = reflection
            .varyings
            .iter()
            .map(|varying| (varying.location, varying.component, varying.count))
            .collect();
        assert_eq!(
            read,
            [(0, ComponentType::Uint, 1), (1, ComponentType::Uint, 2), (2, ComponentType::Int, 3)]
        );
        for (ty, location, says) in [
            (FREE + 11, 0, "the varying at location 0 is a bool, where a varying is a float"),
            (FREE + 12, 0, "the varying at location 0 is a uint of 64 bits"),
            (FREE + 13, 0, "the varying at location 0 is a float of 16 bits"),
            (FREE + 16, 0, "the varying at location 0 is a vector of 2 of another type"),
            (UINT, 16, "the varying at location 16, where a varying is below location 16"),
        ] {
            let mut globals = types.to_vec();
            globals.extend(stage_variable(OUTPUT, ty, &[&[LOCATION, location]]));
            refused(&assemble(VERTEX, &[FREE + 1], &globals, &[]), says);
        }
    }

    #[test]
    fn a_varying_packed_by_component_or_centroid_or_sample_is_refused() {
        for (decoration, says) in [
            (&[COMPONENT, 2][..], "the varying at location 1 has a `Component` decoration"),
            (&[CENTROID], "the varying at location 1 is `centroid`, which the cook refuses"),
            (&[SAMPLE], "the varying at location 1 is `sample`, which the cook refuses"),
        ] {
            let read = stage_variable(INPUT, VEC4, &[&[LOCATION, 1], decoration]);
            refused(&assemble(FRAGMENT, &[FREE + 1], &read, &[]), says);
            let written = stage_variable(OUTPUT, VEC4, &[&[LOCATION, 1], decoration]);
            refused(&assemble(VERTEX, &[FREE + 1], &written, &[]), says);
        }
        let output = stage_variable(OUTPUT, VEC4, &[&[LOCATION, 0], &[COMPONENT, 0]]);
        refused(
            &assemble(FRAGMENT, &[FREE + 1], &output, &[]),
            "the fragment output at location 0 has a `Component` decoration",
        );
    }

    /// A module of `model` whose interface is a `VEC4` varying of `storage` at each location
    /// below `locations` and a built-in of each of `built_ins`, its type and its `BuiltIn`.
    fn interface(model: u32, storage: u32, locations: u32, built_ins: &[(u32, u32)]) -> Vec<u8> {
        let mut globals = vec![
            (OP_TYPE_BOOL, vec![FREE]),
            (OP_TYPE_ARRAY, vec![FREE + 1, FLOAT, FREE + 2]),
            (OP_CONSTANT, vec![UINT, FREE + 2, 2]),
            (OP_TYPE_POINTER, vec![FREE + 3, storage, VEC4]),
        ];
        let mut interface = Vec::new();
        for location in 0..locations {
            let variable = FREE + 10 + location;
            globals.push((OP_VARIABLE, vec![FREE + 3, variable, storage]));
            globals.push((OP_DECORATE, vec![variable, LOCATION, location]));
            interface.push(variable);
        }
        for (at, &(ty, built_in)) in (0..).zip(built_ins) {
            let (pointer, variable) = (FREE + 40 + 2 * at, FREE + 41 + 2 * at);
            globals.extend([
                (OP_TYPE_POINTER, vec![pointer, storage, ty]),
                (OP_VARIABLE, vec![pointer, variable, storage]),
                (OP_DECORATE, vec![variable, BUILT_IN, built_in]),
            ]);
            interface.push(variable);
        }
        assemble(model, &interface, &globals, &[])
    }

    #[test]
    fn the_varyings_and_the_built_ins_beside_them_take_at_most_64_components() {
        // `Position` and `FragCoord` a `VEC4`, `FrontFacing` a bool, `ClipDistance` two floats.
        let (position, frag_coord, front_facing, clip) =
            ((VEC4, 0), (VEC4, 15), (FREE, 17), (FREE + 1, 3));
        let count = |code: &[u8]| reflect(code).map(|reflection| reflection.varyings.len());
        assert_eq!(count(&interface(VERTEX, OUTPUT, 15, &[position])).unwrap(), 15);
        assert_eq!(count(&interface(FRAGMENT, INPUT, 15, &[frag_coord])).unwrap(), 15);
        assert_eq!(count(&interface(VERTEX, OUTPUT, 16, &[])).unwrap(), 16);
        assert_eq!(count(&interface(VERTEX, OUTPUT, 14, &[position, clip])).unwrap(), 14);
        for (code, says) in [
            (
                interface(VERTEX, OUTPUT, 16, &[position]),
                "the vertex entry's outputs take 68 components, 64 for the locations up to the \
                 highest and 4 for the built-ins",
            ),
            (
                interface(FRAGMENT, INPUT, 15, &[frag_coord, front_facing]),
                "the fragment entry's inputs take 65 components, 60 for the locations up to the \
                 highest and 5 for the built-ins",
            ),
            (
                interface(VERTEX, OUTPUT, 15, &[position, clip]),
                "the vertex entry's outputs take 66 components, 60 for the locations up to the \
                 highest and 6 for the built-ins",
            ),
        ] {
            refused(&code, says);
        }
        let error = reflect(&interface(VERTEX, OUTPUT, 16, &[position])).unwrap_err();
        assert!(error.message().contains("VUID-RuntimeSpirv-Location-06272"), "{error}");
    }

    #[test]
    fn a_compute_entrys_workgroup_is_read_and_one_of_ids_or_out_of_bounds_is_refused() {
        // The module `assemble` gives of `model`, with `modes` in place of its own.
        let with_modes = |model: u32, modes: &[(u16, &[u32])]| {
            let code = assemble(model, &[], &[], &[]);
            let mut words: Vec<u32> =
                code.as_chunks::<4>().0.iter().map(|w| u32::from_le_bytes(*w)).collect();
            // After the header, the capability and the entry point.
            let at = 7 + (words[7] >> 16) as usize;
            if words[at] & 0xffff == u32::from(OP_EXECUTION_MODE) {
                words.drain(at..at + (words[at] >> 16) as usize);
            }
            let mut inserted = Vec::new();
            for (opcode, operands) in modes {
                let count = u32::try_from(operands.len() + 1).unwrap();
                inserted.push(count << 16 | u32::from(*opcode));
                inserted.extend_from_slice(operands);
            }
            words.splice(at..at, inserted);
            words.iter().flat_map(|word| word.to_le_bytes()).collect::<Vec<u8>>()
        };
        let size =
            |x, y, z| with_modes(COMPUTE, &[(OP_EXECUTION_MODE, &[MAIN, LOCAL_SIZE, x, y, z])]);
        assert_eq!(reflect(&assemble(COMPUTE, &[], &[], &[])).unwrap().workgroup, Some([1, 1, 1]));
        assert_eq!(reflect(&size(64, 2, 1)).unwrap().workgroup, Some([64, 2, 1]));
        assert_eq!(reflect(&size(1, 2, 64)).unwrap().workgroup, Some([1, 2, 64]));
        assert_eq!(reflect(&size(128, 1, 1)).unwrap().workgroup, Some([128, 1, 1]));
        for (x, y, z) in [(0, 1, 1), (1, 1, 0), (129, 1, 1), (1, 129, 1), (1, 1, 65), (64, 2, 2)] {
            let says = format!("the entry's workgroup is [{x}, {y}, {z}], where each dimension");
            refused(&size(x, y, z), &says);
        }
        // A vertex or fragment entry has none.
        assert_eq!(reflect(&assemble(VERTEX, &[], &[], &[])).unwrap().workgroup, None);
        refused(&with_modes(COMPUTE, &[]), "a compute entry with 0 `LocalSize` modes");
        let local: (u16, &[u32]) = (OP_EXECUTION_MODE, &[MAIN, LOCAL_SIZE, 1, 1, 1]);
        refused(&with_modes(COMPUTE, &[local, local]), "a compute entry with 2 `LocalSize` modes");
        let by_id: (u16, &[u32]) = (OP_EXECUTION_MODE_ID, &[MAIN, LOCAL_SIZE_ID, N0, N0, N0]);
        let says = "`LocalSizeId`, of ids a specialisation may change";
        refused(&with_modes(COMPUTE, &[by_id]), says);
        refused(&with_modes(COMPUTE, &[local, by_id]), says);
        refused(&with_modes(FRAGMENT, &[local]), "a `LocalSize` outside a compute entry");
        // A constant decorated `BuiltIn WorkgroupSize` would override the `LocalSize`.
        let decorated = [(OP_DECORATE, vec![N1024, BUILT_IN, WORKGROUP_SIZE])];
        refused(
            &assemble(COMPUTE, &[], &decorated, &[]),
            "an id decorated `BuiltIn WorkgroupSize`, which sets the workgroup in place of",
        );
    }

    #[test]
    fn a_vertex_or_compute_input_that_is_not_a_built_in_is_refused() {
        for (model, stage) in [(VERTEX, "vertex"), (COMPUTE, "compute")] {
            let located = stage_variable(INPUT, VEC4, &[&[LOCATION, 0]]);
            let says =
                format!("a {stage} entry's input at location 0, %41, that is not a built-in");
            refused(&assemble(model, &[FREE + 1], &located, &[]), &says);
            let bare = stage_variable(INPUT, VEC4, &[]);
            refused(&assemble(model, &[FREE + 1], &bare, &[]), "input with no location, %41");
            // A built-in, and a block of built-ins, are read.
            let built_in = stage_variable(INPUT, UINT, &[&[BUILT_IN, 42]]);
            reflect(&assemble(model, &[FREE + 1], &built_in, &[])).unwrap();
            let mut block = stage_variable(INPUT, FREE + 2, &[]);
            block.extend([
                (OP_TYPE_STRUCT, vec![FREE + 2, VEC4]),
                (OP_MEMBER_DECORATE, vec![FREE + 2, 0, BUILT_IN, 0]),
            ]);
            reflect(&assemble(model, &[FREE + 1], &block, &[])).unwrap();
        }
    }

    #[test]
    fn the_constants_are_read_and_each_has_an_id_and_a_type() {
        let bool_ = (OP_TYPE_BOOL, vec![FREE]);
        let long = (OP_TYPE_INT, vec![FREE + 1, 64, 0]);
        let int = (OP_TYPE_INT, vec![FREE + 5, 32, 1]);
        let constants = vec![
            bool_,
            int,
            (OP_SPEC_CONSTANT_TRUE, vec![FREE, FREE + 2]),
            (OP_DECORATE, vec![FREE + 2, SPEC_ID, 7]),
            (OP_SPEC_CONSTANT, vec![FLOAT, FREE + 3, 0]),
            (OP_DECORATE, vec![FREE + 3, SPEC_ID, 2]),
            (OP_SPEC_CONSTANT, vec![UINT, FREE + 6, 3]),
            (OP_DECORATE, vec![FREE + 6, SPEC_ID, 9]),
            (OP_SPEC_CONSTANT, vec![FREE + 5, FREE + 7, u32::MAX]),
            (OP_DECORATE, vec![FREE + 7, SPEC_ID, 4]),
            // A composite or an operation of constants has no id of its own.
            (OP_SPEC_CONSTANT_OP, vec![UINT, FREE + 4, 128, FREE + 3]),
        ];
        let reflection = reflect(&assemble(FRAGMENT, &[], &constants, &[])).unwrap();
        let expected = vec![
            ConstantSlot { id: 2, ty: ConstantType::Float },
            ConstantSlot { id: 4, ty: ConstantType::Int },
            ConstantSlot { id: 7, ty: ConstantType::Bool },
            ConstantSlot { id: 9, ty: ConstantType::Uint },
        ];
        assert_eq!(reflection.constants, expected);
        for (globals, says) in [
            (
                vec![(OP_SPEC_CONSTANT, vec![UINT, FREE + 2, 0])],
                "a specialisation constant, %42, with no id",
            ),
            (
                vec![
                    long,
                    (OP_SPEC_CONSTANT, vec![FREE + 1, FREE + 2, 0, 0]),
                    (OP_DECORATE, vec![FREE + 2, SPEC_ID, 1]),
                ],
                "specialisation constant 1 is a uint of 64 bits, where the format holds a bool, \
                 an int, a uint or a float, the last three of 32 bits",
            ),
            (
                vec![
                    (OP_SPEC_CONSTANT, vec![UINT, FREE + 2, 0]),
                    (OP_DECORATE, vec![FREE + 2, SPEC_ID, 1]),
                    (OP_SPEC_CONSTANT, vec![UINT, FREE + 3, 0]),
                    (OP_DECORATE, vec![FREE + 3, SPEC_ID, 1]),
                ],
                "two specialisation constants share an id",
            ),
        ] {
            refused(&assemble(FRAGMENT, &[], &globals, &[]), says);
        }
    }

    /// A push block of `members` at `offsets`, its struct `FREE + 10`, with `globals`.
    fn push_block(members: &[u32], offsets: &[u32], globals: &[(u16, Vec<u32>)]) -> Vec<u8> {
        let block = FREE + 10;
        let mut struct_ = vec![block];
        struct_.extend_from_slice(members);
        let mut all = vec![
            (OP_TYPE_STRUCT, struct_),
            (OP_TYPE_POINTER, vec![block + 1, PUSH_CONSTANT, block]),
            (OP_VARIABLE, vec![block + 1, block + 2, PUSH_CONSTANT]),
        ];
        for (member, &offset) in (0u32..).zip(offsets) {
            all.push((OP_MEMBER_DECORATE, vec![block, member, OFFSET, offset]));
        }
        all.extend_from_slice(globals);
        assemble(FRAGMENT, &[], &all, &[])
    }

    #[test]
    fn the_push_block_is_sized_to_its_last_member() {
        let pointer = (OP_TYPE_POINTER, vec![FREE, PHYSICAL_STORAGE_BUFFER, VEC3]);
        let bytes = |code: &[u8]| reflect(code).unwrap().push_bytes;
        assert_eq!(bytes(&push_block(&[VEC4, FLOAT], &[0, 16], &[])), 20);
        // A device address is 8 bytes, whatever it points to.
        assert_eq!(bytes(&push_block(&[FREE, UINT], &[0, 8], &[pointer])), 12);
        // An array is its stride a element; a matrix its stride a column, or a row if it is
        // row-major.
        let array = [
            (OP_TYPE_ARRAY, vec![FREE, UINT, FREE + 1]),
            (OP_CONSTANT, vec![UINT, FREE + 1, 3]),
            (OP_DECORATE, vec![FREE, ARRAY_STRIDE, 4]),
        ];
        assert_eq!(bytes(&push_block(&[FREE], &[0], &array)), 12);
        let matrix = (OP_TYPE_MATRIX, vec![FREE, VEC4, 2]);
        let stride = (OP_MEMBER_DECORATE, vec![FREE + 10, 0, MATRIX_STRIDE, 16]);
        assert_eq!(bytes(&push_block(&[FREE], &[0], &[matrix.clone(), stride.clone()])), 32);
        let row_major = (OP_MEMBER_DECORATE, vec![FREE + 10, 0, ROW_MAJOR]);
        assert_eq!(
            bytes(&push_block(&[FREE], &[0], &[matrix, stride.clone(), row_major.clone()])),
            64
        );
        // A matrix of 3 rows or columns is its stride, 16 on both targets, a row or column:
        // its 3-component vectors are not refused.
        for (column, count, row_major_bytes, column_major_bytes) in
            [(VEC3, 4, 48, 64), (VEC4, 3, 64, 48), (VEC3, 3, 48, 48)]
        {
            let matrix = (OP_TYPE_MATRIX, vec![FREE, column, count]);
            let column_major = [matrix.clone(), stride.clone()];
            assert_eq!(bytes(&push_block(&[FREE], &[0], &column_major)), column_major_bytes);
            let row = [matrix, stride.clone(), row_major.clone()];
            assert_eq!(bytes(&push_block(&[FREE], &[0], &row)), row_major_bytes);
        }
        assert_eq!(bytes(&assemble(FRAGMENT, &[], &[], &[])), 0);
    }

    #[test]
    fn a_push_block_the_targets_lay_out_apart_or_out_of_bounds_is_refused() {
        let three = "the push-constant block holds a 3-component vector, which the SPIR-V lays \
                     out in 12 bytes and the MSL in 16";
        refused(&push_block(&[FLOAT, VEC3], &[0, 16], &[]), three);
        // In a struct, an array or a matrix.
        let inner = [
            (OP_TYPE_STRUCT, vec![FREE, FLOAT, VEC3]),
            (OP_MEMBER_DECORATE, vec![FREE, 0, OFFSET, 0]),
            (OP_MEMBER_DECORATE, vec![FREE, 1, OFFSET, 4]),
        ];
        refused(&push_block(&[FREE], &[0], &inner), three);
        let array =
            [(OP_TYPE_ARRAY, vec![FREE, VEC3, N1024]), (OP_DECORATE, vec![FREE, ARRAY_STRIDE, 16])];
        refused(&push_block(&[FREE], &[0], &array), three);
        // A device address to one is not in the block.
        let pointer = (OP_TYPE_POINTER, vec![FREE, PHYSICAL_STORAGE_BUFFER, VEC3]);
        reflect(&push_block(&[FREE], &[0], &[pointer])).unwrap();
        let bool_ = (OP_TYPE_BOOL, vec![FREE]);
        refused(
            &push_block(&[FREE], &[0], &[bool_]),
            "the push-constant block holds type %40, which the cook cannot size",
        );
        refused(&push_block(&[FLOAT], &[], &[]), "which the cook cannot size");
        // A matrix or an array without its stride.
        let matrix = (OP_TYPE_MATRIX, vec![FREE, VEC4, 4]);
        refused(&push_block(&[FREE], &[0], &[matrix]), "which the cook cannot size");
        let array = (OP_TYPE_ARRAY, vec![FREE, UINT, N1024]);
        refused(&push_block(&[FREE], &[0], &[array]), "which the cook cannot size");
        let max = PUSH_BYTES_MAX;
        let says =
            format!("a push-constant block of {} bytes, above PUSH_BYTES_MAX ({max})", max + 4);
        refused(&push_block(&[FLOAT], &[max], &[]), &says);
        let second = [(OP_VARIABLE, vec![FREE + 11, FREE + 13, PUSH_CONSTANT])];
        refused(&push_block(&[FLOAT], &[0], &second), "more than one push-constant block");
    }
}
