//! Pipelines: what they are made from, the driver cache's bytes and the statistics a
//! device keeps of both.
//!
//! A pipeline has no layout object. Every pipeline's layout is the same shape: the heap's
//! block, the input attachments of its fragment stage and one push-constant range, each
//! as its cooked shaders declare them.

use core::time::Duration;

use lactose_core::{Error, ErrorKind, Handle, Result, xxh64};

use crate::resource::{Format, kind};
use crate::shader::{ConstantType, CookedShader, Reader};

/// A graphics pipeline of a device.
pub type GraphicsPipelineHandle = Handle<kind::GraphicsPipeline>;
/// A compute pipeline of a device.
pub type ComputePipelineHandle = Handle<kind::ComputePipeline>;

/// A value for one specialisation constant.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Constant {
    /// The constant's id, one the shader declares.
    pub id: u32,
    /// Its value, of the type the shader declares for it.
    pub value: ConstantValue,
}

/// A specialisation constant's value, of one of the types a shader declares one of
/// ([`ConstantType`]). How it reaches the driver is the backend's: Vulkan widens a `Bool` to
/// a four-byte `VkBool32`, Metal takes each as the `MTLDataType` of its type.
///
/// Two values are equal when their types and their bits are: a `Float` of `-0.0` is another
/// value than one of `0.0`, and a NaN equals itself, as a pipeline specialised with either
/// does.
#[derive(Clone, Copy, Debug)]
pub enum ConstantValue {
    /// A `Bool`.
    Bool(bool),
    /// An `Int`.
    Int(i32),
    /// A `Uint`.
    Uint(u32),
    /// A `Float`.
    Float(f32),
}

impl ConstantValue {
    /// The type of a constant this is a value of.
    #[must_use]
    pub const fn ty(self) -> ConstantType {
        match self {
            Self::Bool(_) => ConstantType::Bool,
            Self::Int(_) => ConstantType::Int,
            Self::Uint(_) => ConstantType::Uint,
            Self::Float(_) => ConstantType::Float,
        }
    }

    /// The value's bits, a `Bool`'s 0 or 1.
    const fn bits(self) -> u32 {
        match self {
            Self::Bool(value) => value as u32,
            Self::Int(value) => value.cast_unsigned(),
            Self::Uint(value) => value,
            Self::Float(value) => value.to_bits(),
        }
    }
}

impl PartialEq for ConstantValue {
    fn eq(&self, other: &Self) -> bool {
        self.ty() == other.ty() && self.bits() == other.bits()
    }
}

impl Eq for ConstantValue {}

impl core::hash::Hash for ConstantValue {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.ty().hash(state);
        self.bits().hash(state);
    }
}

/// One stage of a pipeline.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StageDesc<'a> {
    /// The cooked shader.
    pub shader: &'a CookedShader,
    /// Values for its constants, each id at most once and each of the type the shader
    /// declares for it. A constant with no value keeps the default the shader gives it.
    pub constants: &'a [Constant],
}

/// Which channels of a colour attachment a pipeline writes: a set of the four bits.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct WriteMask(pub u8);

impl WriteMask {
    /// No channel.
    pub const NONE: Self = Self(0);
    /// The red channel.
    pub const RED: Self = Self(1);
    /// The green channel.
    pub const GREEN: Self = Self(2);
    /// The blue channel.
    pub const BLUE: Self = Self(4);
    /// The alpha channel.
    pub const ALPHA: Self = Self(8);
    /// Every channel.
    pub const ALL: Self = Self(15);
}

/// One colour attachment as a pipeline sees it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ColourTargetDesc {
    /// The attachment's format.
    pub format: Format,
    /// The channels the pipeline writes. Each attachment has its own.
    pub write_mask: WriteMask,
}

/// A comparison of a fragment's depth with the attachment's.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Compare {
    /// Never passes.
    Never,
    /// Passes when the fragment's is less.
    Less,
    /// Passes when the fragment's is less or equal.
    LessEqual,
    /// Passes when the two are equal.
    Equal,
    /// Passes when the fragment's is greater or equal.
    GreaterEqual,
    /// Passes when the fragment's is greater.
    Greater,
    /// Always passes.
    Always,
}

/// The depth attachment as a pipeline sees it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct DepthDesc {
    /// The attachment's format, a depth format.
    pub format: Format,
    /// The test.
    pub compare: Compare,
    /// Whether a fragment that passes writes its depth.
    pub write: bool,
}

/// Which side of a triangle is not drawn. A triangle's front is the side on which its
/// vertices run anticlockwise as the image is displayed (`README.md`, "Render passes").
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Cull {
    /// Both sides are drawn.
    None,
    /// The front is not drawn.
    Front,
    /// The back is not drawn.
    Back,
}

/// What a graphics pipeline is made from. It draws triangle lists, and reads its vertices
/// through device addresses: there is no vertex buffer binding (architecture section
/// 12.4).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GraphicsPipelineDesc<'a> {
    /// A name for traces and for the driver's debug tools.
    pub label: &'a str,
    /// The vertex stage.
    pub vertex: StageDesc<'a>,
    /// The fragment stage, or `None` for a pipeline that writes depth alone.
    pub fragment: Option<StageDesc<'a>>,
    /// The colour attachments of the passes it draws in, in order.
    pub colour: &'a [ColourTargetDesc],
    /// The depth attachment of the passes it draws in, or `None` for a pass with none.
    pub depth: Option<DepthDesc>,
    /// The side not drawn.
    pub cull: Cull,
}

/// What a compute pipeline is made from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ComputePipelineDesc<'a> {
    /// A name for traces and for the driver's debug tools.
    pub label: &'a str,
    /// The compute stage.
    pub stage: StageDesc<'a>,
}

/// How the driver's pipeline cache started.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CacheStart {
    /// The request gave no bytes.
    Empty,
    /// The request's bytes passed every check and the driver took them.
    Loaded,
    /// The request's bytes failed the facade's length or checksum, the backend's identity
    /// check, or the driver's own load: the cache started empty.
    Rejected,
}

/// What a device has done with pipelines. Counts only grow.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PipelineStats {
    /// How the cache started.
    pub cache: CacheStart,
    /// Pipelines created.
    pub pipelines: u64,
    /// Stages the driver's cache had, where the driver says so. A statistic: the test of
    /// a warm cache is the suite's (`README.md`, "The pipeline cache").
    pub stage_hits: u64,
    /// Stages the driver's cache did not have, where the driver says so.
    pub stage_misses: u64,
    /// The time spent inside pipeline creation, summed.
    pub creation: Duration,
}

/// What identifies the driver a cache was written for. A backend builds it from what its
/// driver reports: on Vulkan the vendor, the device and the cache's own identifier; on
/// Metal the system's build, the device, its family and its architecture's name.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CacheIdentity<'a>(pub &'a [u8]);

/// Frames a driver's cache payload as the bytes
/// [`Device::pipeline_cache`](crate::Device::pipeline_cache) returns: a magic number, a
/// version, the identity, the payload's length, the payload, and a checksum over all of
/// it. For backends. [`open_cache`] reads every cache this seals.
///
/// # Errors
///
/// `ErrorKind::InvalidData` for an identity of 4 GiB or more, which the framing's
/// four-byte length cannot hold.
pub fn seal_cache(identity: CacheIdentity<'_>, payload: &[u8]) -> Result<Vec<u8>> {
    let identity_len = u32::try_from(identity.0.len()).map_err(|_| {
        Error::new(ErrorKind::InvalidData, "sealed cache: an identity of 4 GiB or more")
    })?;
    let mut out = Vec::with_capacity(CACHE_MAGIC.len() + 24 + identity.0.len() + payload.len());
    out.extend_from_slice(&CACHE_MAGIC);
    out.extend_from_slice(&CACHE_VERSION.to_le_bytes());
    out.extend_from_slice(&identity_len.to_le_bytes());
    out.extend_from_slice(identity.0);
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(payload);
    let sum = xxh64(&out, 0);
    out.extend_from_slice(&sum.to_le_bytes());
    Ok(out)
}

/// The first bytes of a sealed cache.
const CACHE_MAGIC: [u8; 4] = *b"LPCS";

/// The version [`seal_cache`] writes and [`open_cache`] reads.
const CACHE_VERSION: u32 = 1;

/// The payload inside `bytes`, exactly as it was sealed. For backends: the bytes it
/// returns are the only ones a driver is ever handed.
///
/// # Errors
///
/// `ErrorKind::InvalidData` when `bytes` are not a sealed cache, are truncated or longer
/// than their length says, or fail the checksum; `ErrorKind::VersionMismatch` when they
/// are another version's or were sealed under another identity.
pub fn open_cache<'a>(bytes: &'a [u8], identity: CacheIdentity<'_>) -> Result<&'a [u8]> {
    let invalid = |what: &str| Error::new(ErrorKind::InvalidData, format!("sealed cache: {what}"));
    if bytes.len() < CACHE_MAGIC.len() + 4 || bytes[..CACHE_MAGIC.len()] != CACHE_MAGIC {
        return Err(invalid("not a sealed cache"));
    }
    let mut reader = Reader { bytes: &bytes[CACHE_MAGIC.len()..] };
    let version = reader.u32()?;
    if version != CACHE_VERSION {
        return Err(Error::new(
            ErrorKind::VersionMismatch,
            format!("sealed cache of version {version}; this build reads {CACHE_VERSION}"),
        ));
    }
    let truncated = |_| invalid("truncated");
    let identity_len = usize::try_from(reader.u32().map_err(truncated)?).unwrap_or(usize::MAX);
    let sealed_identity = reader.take(identity_len).map_err(truncated)?;
    let payload_len = usize::try_from(reader.u64().map_err(truncated)?).unwrap_or(usize::MAX);
    let payload = reader.take(payload_len).map_err(truncated)?;
    let sum = reader.u64().map_err(truncated)?;
    if !reader.bytes.is_empty() {
        return Err(invalid("longer than its length says"));
    }
    if xxh64(&bytes[..bytes.len() - 8], 0) != sum {
        return Err(invalid("checksum"));
    }
    if sealed_identity != identity.0 {
        return Err(Error::new(ErrorKind::VersionMismatch, "sealed cache of another driver"));
    }
    Ok(payload)
}
