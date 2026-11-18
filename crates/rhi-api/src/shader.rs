//! The cooked shader format: what the cooker writes for one stage and one target, and
//! what a backend makes a pipeline from. It names the heap's shader form.
//!
//! The bytes are [`CookedShader::encode`]'s and nothing else writes them, so the layout
//! is this crate's alone; what it carries is the struct. The cooker checks what the format
//! cannot (`README.md`, "The cooked shader format").

use lactose_core::{Error, ErrorKind, Result, xxh64};

/// The version [`CookedShader::encode`] writes. A change to the struct or to what a field
/// means is a new version, and [`CookedShader::decode`] reads no other.
pub const COOKED_SHADER_VERSION: u32 = 2;

/// The descriptor set (SPIR-V) the heap's arrays are bound in, on every pipeline. The
/// input attachments of a fragment stage are set 0, input `i` at binding `i`.
pub const HEAP_SET: u32 = 1;

/// The bytes of a pipeline's push-constant range at most.
pub const PUSH_BYTES_MAX: u32 = 128;

/// The invocations of a compute stage's workgroup at most in x, y and z: Vulkan's least
/// `maxComputeWorkGroupSize`, within every Apple family's (`README.md`, "Pipelines and the
/// cooked shader format").
pub const WORKGROUP_SIZE_MAX: [u32; 3] = [128, 128, 64];

/// The invocations of a compute stage's workgroup at most, the product of its three
/// dimensions: Vulkan's least `maxComputeWorkGroupInvocations`.
pub const WORKGROUP_INVOCATIONS_MAX: u32 = 128;

/// The locations a varying takes, 0 to 15. The bound is the format's, and no more than
/// Vulkan's least `maxVertexOutputComponents` and `maxFragmentInputComponents`, 64, allow:
/// sixteen locations of four components. Those 64 count the built-ins of the interface too,
/// so a stage that writes `Position` has room for fifteen; the cook holds each stage to the
/// 64 (`README.md`, "Pipelines and the cooked shader format").
pub const VARYING_LOCATIONS: u32 = 16;

/// A pipeline stage.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ShaderStage {
    /// The vertex stage.
    Vertex,
    /// The fragment stage.
    Fragment,
    /// The compute stage.
    Compute,
}

/// What a cooked shader's code is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ShaderTarget {
    /// A SPIR-V module.
    SpirV,
    /// Metal Shading Language 3.1 source, which the backend compiles.
    Msl,
    /// A Metal library, compiled on macOS for macOS 14.
    MetalLibrary,
}

/// The form the heap takes in a shader. One form exists: a later one is a new name and a
/// new [`COOKED_SHADER_VERSION`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HeapForm {
    /// Fixed-size arrays in one parameter block, indexed by a slot number that is the same
    /// on every target (`docs/results/spike-metal-apple-m1-2026-11-17.md`, "The heap's
    /// shader form"). On SPIR-V the block is descriptor set [`HEAP_SET`]; on Metal it is
    /// one argument buffer.
    FixedArray,
}

/// What a heap array holds.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum HeapArrayKind {
    /// Two-dimensional sampled images: the heap's slots
    /// ([`HeapSlot`](crate::HeapSlot)).
    SampledImage2d,
    /// Samplers: the sampler table ([`SamplerIndex`](crate::SamplerIndex)).
    Sampler,
}

/// One array of the heap as a shader declares it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct HeapArray {
    /// What it holds.
    pub kind: HeapArrayKind,
    /// Its binding in the heap's block: 0 for the images, 1 for the samplers.
    pub binding: u32,
    /// Its length: the device's [`heap_slots`](crate::Capabilities::heap_slots) for the
    /// images and its [`samplers`](crate::Capabilities::samplers) for the samplers. A
    /// pipeline is not made from a shader compiled for another length.
    pub capacity: u32,
}

/// How a shader reaches the heap.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct HeapUse {
    /// The form.
    pub form: HeapForm,
    /// The set the block is bound in: [`HEAP_SET`].
    pub set: u32,
    /// The arrays the shader declares, each kind at most once.
    pub arrays: Vec<HeapArray>,
}

/// The type of a specialisation constant.
///
/// A pipeline's value for it is a [`ConstantValue`](crate::ConstantValue) of the same
/// type. A backend hands the driver a value of exactly this type: on Metal, of the
/// `MTLDataType` of the same name, since Metal refuses a value of another type than the
/// function constant's; on Vulkan, a `Bool` widened to a four-byte `VkBool32` and the others
/// as they are (VUID-VkSpecializationMapEntry-constantID-00776).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ConstantType {
    /// A boolean: SPIR-V's `OpTypeBool`, MSL's `bool`.
    Bool,
    /// A signed 32-bit integer: `OpTypeInt 32 1`, `int`.
    Int,
    /// An unsigned 32-bit integer: `OpTypeInt 32 0`, `uint`.
    Uint,
    /// A 32-bit float: `OpTypeFloat 32`, `float`.
    Float,
}

/// A specialisation constant a shader declares.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ConstantSlot {
    /// Its id: the SPIR-V constant id, and the Metal function-constant index.
    pub id: u32,
    /// Its type.
    pub ty: ConstantType,
}

/// The type of a varying's components, each 32 bits.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ComponentType {
    /// A float: SPIR-V's `OpTypeFloat 32`, MSL's `float`.
    Float,
    /// A signed integer: `OpTypeInt 32 1`, `int`.
    Int,
    /// An unsigned integer: `OpTypeInt 32 0`, `uint`.
    Uint,
}

/// A value the vertex stage writes for the fragment stage to read, the built-ins aside: on
/// SPIR-V the variable decorated `Location`, on Metal the field of attribute
/// `[[user(TEXCOORD_n)]]` (Apple's `[[user(locn)]]`; location 0 is Slang's
/// `[[user(TEXCOORD)]]`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Varying {
    /// Its location, below [`VARYING_LOCATIONS`]. It takes this one alone.
    pub location: u32,
    /// The type of its components.
    pub component: ComponentType,
    /// Its components: 1 for a scalar, a vector's 2 to 4.
    pub count: u32,
}

/// One cooked shader: one stage, for one target.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct CookedShader {
    /// The stage.
    pub stage: ShaderStage,
    /// What `code` is.
    pub target: ShaderTarget,
    /// The entry point's name in `code`.
    pub entry: String,
    /// The module, the source or the library.
    pub code: Vec<u8>,
    /// How the shader reaches the heap, or `None` when it does not.
    pub heap: Option<HeapUse>,
    /// The specialisation constants it declares, in ascending id order.
    pub constants: Vec<ConstantSlot>,
    /// The bytes of its push-constant block, at most [`PUSH_BYTES_MAX`].
    pub push_bytes: u32,
    /// Bit `i` is set when the stage reads input attachment `i`, which is colour
    /// attachment `i` of its pass and nothing else. Zero outside the fragment stage.
    pub inputs: u32,
    /// Bit `i` is set when the stage writes colour attachment `i`. Zero outside the
    /// fragment stage.
    pub outputs: u32,
    /// The size of a compute stage's workgroup in x, y and z: each dimension at least 1 and
    /// at most [`WORKGROUP_SIZE_MAX`]'s, and their product at most
    /// [`WORKGROUP_INVOCATIONS_MAX`]. `Some` in the compute stage and `None` outside it. A
    /// SPIR-V module states it too; on Metal the backend passes it to each dispatch as
    /// `threadsPerThreadgroup`.
    pub workgroup: Option<[u32; 3]>,
    /// The varyings the vertex stage writes, or those the fragment stage reads, in ascending
    /// location order. A fragment stage's are those its code reads: one it declares and
    /// never reads is in neither target's interface. Empty in the compute stage.
    pub varyings: Vec<Varying>,
}

impl CookedShader {
    /// The shader's bytes, for the cooker to write: a magic number,
    /// [`COOKED_SHADER_VERSION`], every field, and a checksum over all of it. Equal
    /// shaders give equal bytes on every host, and [`decode`](Self::decode) reads every
    /// shader this accepts.
    ///
    /// # Errors
    ///
    /// `ErrorKind::InvalidData`, naming the field, for a shader that [`check`](Self::check)
    /// refuses, or with an entry, a module, or a list of 4 GiB or more.
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.check()?;
        let mut out = Vec::with_capacity(64 + self.entry.len() + self.code.len());
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&COOKED_SHADER_VERSION.to_le_bytes());
        out.push(stage_code(self.stage));
        out.push(target_code(self.target));
        put_bytes(&mut out, self.entry.as_bytes(), "the entry")?;
        put_bytes(&mut out, &self.code, "the module")?;
        match &self.heap {
            None => out.push(0),
            Some(heap) => {
                out.push(1);
                out.push(match heap.form {
                    HeapForm::FixedArray => 0,
                });
                out.extend_from_slice(&heap.set.to_le_bytes());
                put_len(&mut out, heap.arrays.len(), "the heap's arrays")?;
                for array in &heap.arrays {
                    out.push(kind_code(array.kind));
                    out.extend_from_slice(&array.binding.to_le_bytes());
                    out.extend_from_slice(&array.capacity.to_le_bytes());
                }
            }
        }
        put_len(&mut out, self.constants.len(), "the constants")?;
        for constant in &self.constants {
            out.extend_from_slice(&constant.id.to_le_bytes());
            out.push(constant_code(constant.ty));
        }
        for word in [self.push_bytes, self.inputs, self.outputs] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        match self.workgroup {
            None => out.push(0),
            Some(size) => {
                out.push(1);
                for dimension in size {
                    out.extend_from_slice(&dimension.to_le_bytes());
                }
            }
        }
        put_len(&mut out, self.varyings.len(), "the varyings")?;
        for varying in &self.varyings {
            out.extend_from_slice(&varying.location.to_le_bytes());
            out.push(component_code(varying.component));
            out.extend_from_slice(&varying.count.to_le_bytes());
        }
        let sum = xxh64(&out, 0);
        out.extend_from_slice(&sum.to_le_bytes());
        Ok(out)
    }

    /// Checks what the fields' documentation states, as [`encode`](Self::encode) and
    /// [`decode`](Self::decode) do. A device backend calls it on each stage before its
    /// driver sees the stage, and [`check_varyings`] on a graphics pipeline's two.
    ///
    /// # Errors
    ///
    /// `ErrorKind::InvalidData`, naming the first rule broken: a heap set other than
    /// [`HEAP_SET`], a heap array kind twice or at another binding than its kind's,
    /// constants out of ascending id order, a push block above [`PUSH_BYTES_MAX`], inputs or
    /// outputs outside the fragment stage, a compute stage without a workgroup or another
    /// stage with one, a workgroup with a dimension of zero or beyond
    /// [`WORKGROUP_SIZE_MAX`] or of more than [`WORKGROUP_INVOCATIONS_MAX`] invocations,
    /// varyings in the compute stage, or varyings out of ascending location order, at a
    /// location from [`VARYING_LOCATIONS`] or of other than 1 to 4 components.
    pub fn check(&self) -> Result<()> {
        match self.breach() {
            Some(what) => Err(Error::new(ErrorKind::InvalidData, format!("cooked shader: {what}"))),
            None => Ok(()),
        }
    }

    /// The first rule of the fields' documentation the shader breaks.
    fn breach(&self) -> Option<&'static str> {
        if let Some(heap) = &self.heap {
            if heap.set != HEAP_SET {
                return Some("a heap set other than HEAP_SET");
            }
            let twice = heap.arrays.iter().enumerate().any(|(at, array)| {
                heap.arrays[..at].iter().any(|before| before.kind == array.kind)
            });
            if twice
                || heap.arrays.iter().any(|array| array.binding != u32::from(kind_code(array.kind)))
            {
                return Some("a heap array of a kind twice, or at another binding than its kind's");
            }
        }
        if !self.constants.windows(2).all(|two| two[0].id < two[1].id) {
            return Some("constants out of ascending id order");
        }
        if self.push_bytes > PUSH_BYTES_MAX {
            return Some("a push block above PUSH_BYTES_MAX");
        }
        if self.stage != ShaderStage::Fragment && (self.inputs != 0 || self.outputs != 0) {
            return Some("inputs or outputs outside the fragment stage");
        }
        match (self.stage, self.workgroup) {
            (ShaderStage::Compute, None) => return Some("a compute stage without a workgroup"),
            (ShaderStage::Compute, Some(size)) => {
                let within =
                    size.iter().zip(WORKGROUP_SIZE_MAX).all(|(&n, most)| (1..=most).contains(&n));
                // Within the bounds the product cannot overflow.
                if !within || size.iter().product::<u32>() > WORKGROUP_INVOCATIONS_MAX {
                    return Some(
                        "a workgroup with a dimension of zero or above WORKGROUP_SIZE_MAX, or of \
                         more than WORKGROUP_INVOCATIONS_MAX invocations",
                    );
                }
            }
            (_, Some(_)) => return Some("a workgroup outside the compute stage"),
            (_, None) => {}
        }
        if self.stage == ShaderStage::Compute && !self.varyings.is_empty() {
            return Some("varyings in the compute stage");
        }
        let ascending = self.varyings.windows(2).all(|two| two[0].location < two[1].location);
        let each = self.varyings.iter().all(|varying| {
            varying.location < VARYING_LOCATIONS && (1..=4).contains(&varying.count)
        });
        if !ascending || !each {
            return Some(
                "varyings out of ascending location order, at a location from VARYING_LOCATIONS, \
                 or of other than 1 to 4 components",
            );
        }
        None
    }

    /// Reads what [`encode`](Self::encode) wrote.
    ///
    /// # Errors
    ///
    /// `ErrorKind::VersionMismatch` for another version's bytes; `ErrorKind::InvalidData`
    /// for bytes that are not a cooked shader, that are truncated or fail the checksum, or
    /// whose shader [`check`](Self::check) refuses.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let invalid =
            |what: &str| Error::new(ErrorKind::InvalidData, format!("cooked shader: {what}"));
        if bytes.len() < MAGIC.len() + 4 + 8 || bytes[..MAGIC.len()] != MAGIC {
            return Err(invalid("not a cooked shader"));
        }
        let mut reader = Reader { bytes: &bytes[MAGIC.len()..] };
        let version = reader.u32()?;
        if version != COOKED_SHADER_VERSION {
            return Err(Error::new(
                ErrorKind::VersionMismatch,
                format!(
                    "cooked shader of version {version}; this build reads {COOKED_SHADER_VERSION}"
                ),
            ));
        }
        let (body, sum) = bytes.split_at(bytes.len() - 8);
        if xxh64(body, 0).to_le_bytes() != sum {
            return Err(invalid("checksum"));
        }
        let mut reader = Reader { bytes: &body[MAGIC.len() + 4..] };
        let stage = match reader.u8()? {
            0 => ShaderStage::Vertex,
            1 => ShaderStage::Fragment,
            2 => ShaderStage::Compute,
            _ => return Err(invalid("stage")),
        };
        let target = match reader.u8()? {
            0 => ShaderTarget::SpirV,
            1 => ShaderTarget::Msl,
            2 => ShaderTarget::MetalLibrary,
            _ => return Err(invalid("target")),
        };
        let entry = String::from_utf8(reader.bytes()?.to_vec()).map_err(|_| invalid("entry"))?;
        let code = reader.bytes()?.to_vec();
        let heap = match reader.u8()? {
            0 => None,
            1 => {
                let form = match reader.u8()? {
                    0 => HeapForm::FixedArray,
                    _ => return Err(invalid("heap form")),
                };
                let set = reader.u32()?;
                let count = reader.len(9)?;
                let mut arrays: Vec<HeapArray> = Vec::with_capacity(count);
                for _ in 0..count {
                    let kind = match reader.u8()? {
                        0 => HeapArrayKind::SampledImage2d,
                        1 => HeapArrayKind::Sampler,
                        _ => return Err(invalid("heap array kind")),
                    };
                    let binding = reader.u32()?;
                    let capacity = reader.u32()?;
                    arrays.push(HeapArray { kind, binding, capacity });
                }
                Some(HeapUse { form, set, arrays })
            }
            _ => return Err(invalid("heap")),
        };
        let count = reader.len(5)?;
        let mut constants: Vec<ConstantSlot> = Vec::with_capacity(count);
        for _ in 0..count {
            let id = reader.u32()?;
            let ty = match reader.u8()? {
                0 => ConstantType::Bool,
                1 => ConstantType::Int,
                2 => ConstantType::Uint,
                3 => ConstantType::Float,
                _ => return Err(invalid("constant type")),
            };
            constants.push(ConstantSlot { id, ty });
        }
        let push_bytes = reader.u32()?;
        let inputs = reader.u32()?;
        let outputs = reader.u32()?;
        let workgroup = match reader.u8()? {
            0 => None,
            1 => Some([reader.u32()?, reader.u32()?, reader.u32()?]),
            _ => return Err(invalid("workgroup")),
        };
        let count = reader.len(9)?;
        let mut varyings: Vec<Varying> = Vec::with_capacity(count);
        for _ in 0..count {
            let location = reader.u32()?;
            let component = match reader.u8()? {
                0 => ComponentType::Float,
                1 => ComponentType::Int,
                2 => ComponentType::Uint,
                _ => return Err(invalid("varying component type")),
            };
            varyings.push(Varying { location, component, count: reader.u32()? });
        }
        if !reader.bytes.is_empty() {
            return Err(invalid("trailing bytes"));
        }
        let shader = Self {
            stage,
            target,
            entry,
            code,
            heap,
            constants,
            push_bytes,
            inputs,
            outputs,
            workgroup,
            varyings,
        };
        shader.check()?;
        Ok(shader)
    }
}

/// Checks the varyings rule between a graphics pipeline's vertex and fragment stages: each
/// varying `fragment` reads is one `vertex` writes, at the same location, of the same
/// component type and count (`README.md`, "Pipelines and the cooked shader format"). A
/// varying written and not read is allowed. A device backend calls it before its driver
/// sees the stages: it is the only guard for a `Uint` read as an `Int`, which Vulkan's
/// validation does not report and Metal makes.
///
/// # Errors
///
/// `ErrorKind::InvalidData`, naming the first location read that is not written, or that is
/// written with another component type or count, and what is written there.
pub fn check_varyings(vertex: &CookedShader, fragment: &CookedShader) -> Result<()> {
    for read in &fragment.varyings {
        let written = vertex.varyings.iter().find(|written| written.location == read.location);
        let what = match written {
            Some(written) if written == read => continue,
            Some(written) => format!("which the vertex stage writes as {}", written.describe()),
            None => "which the vertex stage does not write".to_owned(),
        };
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "a fragment stage reading location {} as {}, {what}",
                read.location,
                read.describe()
            ),
        ));
    }
    Ok(())
}

impl Varying {
    /// The varying's components, as a message names them: `2 Float`.
    fn describe(&self) -> String {
        format!("{} {:?}", self.count, self.component)
    }
}

/// The first bytes of an encoded shader.
pub(crate) const MAGIC: [u8; 4] = *b"LCSH";

const fn stage_code(stage: ShaderStage) -> u8 {
    match stage {
        ShaderStage::Vertex => 0,
        ShaderStage::Fragment => 1,
        ShaderStage::Compute => 2,
    }
}

const fn target_code(target: ShaderTarget) -> u8 {
    match target {
        ShaderTarget::SpirV => 0,
        ShaderTarget::Msl => 1,
        ShaderTarget::MetalLibrary => 2,
    }
}

const fn constant_code(ty: ConstantType) -> u8 {
    match ty {
        ConstantType::Bool => 0,
        ConstantType::Int => 1,
        ConstantType::Uint => 2,
        ConstantType::Float => 3,
    }
}

const fn component_code(component: ComponentType) -> u8 {
    match component {
        ComponentType::Float => 0,
        ComponentType::Int => 1,
        ComponentType::Uint => 2,
    }
}

/// The kind's code, which is also its binding.
const fn kind_code(kind: HeapArrayKind) -> u8 {
    match kind {
        HeapArrayKind::SampledImage2d => 0,
        HeapArrayKind::Sampler => 1,
    }
}

/// Writes a length as four little-endian bytes, or refuses `what` at 4 GiB or more.
fn put_len(out: &mut Vec<u8>, len: usize, what: &str) -> Result<()> {
    let len = u32::try_from(len).map_err(|_| {
        Error::new(
            ErrorKind::InvalidData,
            format!("cooked shader to encode: {what} of 4 GiB or more"),
        )
    })?;
    out.extend_from_slice(&len.to_le_bytes());
    Ok(())
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8], what: &str) -> Result<()> {
    put_len(out, bytes.len(), what)?;
    out.extend_from_slice(bytes);
    Ok(())
}

/// Reads little-endian fields from the front of a byte slice, refusing a short one.
pub(crate) struct Reader<'a> {
    pub(crate) bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.bytes.len() < n {
            return Err(Error::new(ErrorKind::InvalidData, "truncated"));
        }
        let (head, rest) = self.bytes.split_at(n);
        self.bytes = rest;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn u32(&mut self) -> Result<u32> {
        let mut word = [0; 4];
        word.copy_from_slice(self.take(4)?);
        Ok(u32::from_le_bytes(word))
    }

    pub(crate) fn u64(&mut self) -> Result<u64> {
        let mut word = [0; 8];
        word.copy_from_slice(self.take(8)?);
        Ok(u64::from_le_bytes(word))
    }

    /// A count of records of `each` bytes, refused where they cannot fit in what is left.
    fn len(&mut self, each: usize) -> Result<usize> {
        let count = usize::try_from(self.u32()?).unwrap_or(usize::MAX);
        if count.saturating_mul(each) > self.bytes.len() {
            return Err(Error::new(ErrorKind::InvalidData, "truncated"));
        }
        Ok(count)
    }

    fn bytes(&mut self) -> Result<&'a [u8]> {
        let len = self.len(1)?;
        self.take(len)
    }
}
