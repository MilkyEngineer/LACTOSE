//! Buffers, images, memory blocks and samplers: the handles and the descriptors they are
//! made from.
//!
//! A handle is plain data (`lactose_core::Handle`): `Copy`, `Send`, `Sync`, an index and
//! a generation. It means something to the device that issued it and to no other, and a
//! handle whose resource is destroyed is refused wherever it is used
//! ([`Rule::Destroyed`](crate::Rule::Destroyed)), on every backend.

use lactose_core::Handle;

/// The markers the handle types are told apart by. Nothing is ever a value of one.
pub mod kind {
    /// Marks a [`BufferHandle`](super::BufferHandle).
    pub enum Buffer {}
    /// Marks an [`ImageHandle`](super::ImageHandle).
    pub enum Image {}
    /// Marks a [`MemoryBlockHandle`](super::MemoryBlockHandle).
    pub enum MemoryBlock {}
    /// Marks a [`GraphicsPipelineHandle`](crate::GraphicsPipelineHandle).
    pub enum GraphicsPipeline {}
    /// Marks a [`ComputePipelineHandle`](crate::ComputePipelineHandle).
    pub enum ComputePipeline {}
    /// Marks a [`TimelineHandle`](crate::TimelineHandle).
    pub enum Timeline {}
}

/// A buffer of a device.
pub type BufferHandle = Handle<kind::Buffer>;
/// An image of a device, a swapchain's images included.
pub type ImageHandle = Handle<kind::Image>;
/// A block of device memory that images are placed in.
pub type MemoryBlockHandle = Handle<kind::MemoryBlock>;

/// Declares a set of flags over `u32`: the constants, the empty set, union and
/// containment.
macro_rules! flags {
    (
        $(#[$meta:meta])*
        $name:ident { $( $(#[$flag_meta:meta])* $flag:ident = $bit:expr; )+ }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub struct $name(u32);

        impl $name {
            /// No flag.
            pub const EMPTY: Self = Self(0);
            $( $(#[$flag_meta])* pub const $flag: Self = Self(1 << $bit); )+

            /// The flags of `self` and of `other`.
            #[must_use]
            pub const fn union(self, other: Self) -> Self {
                Self(self.0 | other.0)
            }

            /// Whether every flag of `other` is in `self`.
            #[must_use]
            pub const fn contains(self, other: Self) -> bool {
                self.0 & other.0 == other.0
            }

            /// The flags as bits, in declaration order from bit 0.
            #[must_use]
            pub const fn bits(self) -> u32 {
                self.0
            }
        }
    };
}

flags! {
    /// What a buffer may be used for. A use outside it is refused
    /// ([`Rule::Usage`](crate::Rule::Usage)).
    BufferUsage {
        /// A shader reaches it through its device address
        /// ([`Device::buffer_address`](crate::Device::buffer_address)).
        DEVICE_ADDRESS = 0;
        /// The index buffer of a draw. Indices are 32 bits.
        INDEX = 1;
        /// The argument or count buffer of an indirect draw or dispatch.
        INDIRECT = 2;
        /// The source of a copy.
        COPY_SOURCE = 3;
        /// The destination of a copy.
        COPY_DESTINATION = 4;
    }
}

flags! {
    /// What an image may be used for. A use outside it is refused
    /// ([`Rule::Usage`](crate::Rule::Usage)).
    ImageUsage {
        /// A heap slot may hold it, and a shader sample it.
        SAMPLED = 0;
        /// A colour attachment.
        COLOUR = 1;
        /// The depth attachment.
        DEPTH = 2;
        /// A colour attachment its own pass reads through the local read.
        INPUT = 3;
        /// Nothing needs its contents after the pass that writes them: it is an attachment
        /// and nothing else, so the only flags beside this one are `COLOUR`, `DEPTH` and
        /// `INPUT`. A memoryless request needs it.
        TRANSIENT = 4;
        /// The source of a copy.
        COPY_SOURCE = 5;
        /// The destination of a copy.
        COPY_DESTINATION = 6;
    }
}

/// Where a buffer's memory is and who may touch it from the host.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MemoryClass {
    /// Device memory the host never touches: filled by a copy or by a pass that declares it
    /// `ShaderWrite`.
    Device,
    /// Memory the host writes and the device reads. The host may not read it back.
    Upload,
    /// Memory the device writes and the host reads.
    Readback,
}

/// Whether a resource is owned by one queue at a time.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Sharing {
    /// One queue owns it, and a use on another needs a release and an acquire, unless its
    /// contents are discarded.
    Exclusive,
    /// Every queue may use it with no transfer.
    Concurrent,
}

/// What a buffer is made from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BufferDesc<'a> {
    /// A name for traces and for the driver's debug tools.
    pub label: &'a str,
    /// Its size in bytes, above zero.
    pub size: u64,
    /// What it may be used for.
    pub usage: BufferUsage,
    /// Where its memory is.
    pub memory: MemoryClass,
    /// Whether one queue owns it.
    pub sharing: Sharing,
}

/// A pixel format. The set is what milestone 2 draws and presents with; a backend answers
/// one it cannot make with `ErrorKind::Unsupported`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
pub enum Format {
    /// Four 8-bit channels, red first, read as fractions.
    Rgba8Unorm,
    /// As `Rgba8Unorm`, with the sRGB transfer function on the colour channels.
    Rgba8Srgb,
    /// Four 8-bit channels, blue first, read as fractions.
    Bgra8Unorm,
    /// As `Bgra8Unorm`, with the sRGB transfer function on the colour channels.
    Bgra8Srgb,
    /// Four 16-bit floats.
    Rgba16Float,
    /// One 32-bit unsigned integer.
    R32Uint,
    /// One 32-bit float.
    R32Float,
    /// A 32-bit float depth.
    Depth32Float,
}

impl Format {
    /// Whether the format is a depth format, and so a depth attachment's and never a
    /// colour attachment's.
    #[must_use]
    pub const fn is_depth(self) -> bool {
        matches!(self, Self::Depth32Float)
    }

    /// The bytes of one texel, which a copy between a buffer and an image moves.
    #[must_use]
    pub const fn texel_bytes(self) -> u32 {
        match self {
            Self::Rgba16Float => 8,
            Self::Rgba8Unorm
            | Self::Rgba8Srgb
            | Self::Bgra8Unorm
            | Self::Bgra8Srgb
            | Self::R32Uint
            | Self::R32Float
            | Self::Depth32Float => 4,
        }
    }
}

/// A width and a height in texels, each above zero.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Extent {
    /// The width.
    pub width: u32,
    /// The height.
    pub height: u32,
}

/// Where an image's memory comes from.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ImageMemory {
    /// Memory of its own.
    Own,
    /// A request for no memory outside the pass that writes it. The device may decline it
    /// and give the image memory of its own: [`Device::image_memoryless`] says which
    /// happened, and [`Capabilities`](crate::Capabilities) says beforehand. The usage must
    /// hold `TRANSIENT`.
    ///
    /// [`Device::image_memoryless`]: crate::Device::image_memoryless
    Memoryless,
    /// Placed in a block at an offset, where it may overlap other images placed there:
    /// that overlap is the alias relation the backend fences
    /// ([`Encoder::alias`](crate::Encoder::alias)).
    Placed {
        /// The block.
        block: MemoryBlockHandle,
        /// The offset in bytes, a multiple of the image's
        /// [`Placement::alignment`].
        offset: u64,
    },
}

/// What an image is made from: two dimensions, with mips and layers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ImageDesc<'a> {
    /// A name for traces and for the driver's debug tools.
    pub label: &'a str,
    /// The format.
    pub format: Format,
    /// The extent of mip 0.
    pub extent: Extent,
    /// The number of mips, at least one.
    pub mips: u32,
    /// The number of layers, at least one.
    pub layers: u32,
    /// What it may be used for.
    pub usage: ImageUsage,
    /// Where its memory comes from.
    pub memory: ImageMemory,
    /// Whether one queue owns it.
    pub sharing: Sharing,
}

/// What a block must offer an image placed in it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Placement {
    /// The bytes the image takes from its offset.
    pub size: u64,
    /// What its offset must be a multiple of.
    pub alignment: u64,
}

/// What a memory block is made from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MemoryBlockDesc<'a> {
    /// A name for traces and for the driver's debug tools.
    pub label: &'a str,
    /// Its size in bytes, above zero.
    pub size: u64,
}

/// A range of an image's mips and layers.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ImageRange {
    /// The first mip.
    pub mip: u32,
    /// How many mips, at least one.
    pub mips: u32,
    /// The first layer.
    pub layer: u32,
    /// How many layers, at least one.
    pub layers: u32,
}

impl ImageRange {
    /// Mip 0 of layer 0: the whole of an image with one mip and one layer.
    pub const FIRST: Self = Self { mip: 0, mips: 1, layer: 0, layers: 1 };
}

/// How a sampler picks between texels.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Filter {
    /// The nearest texel.
    Nearest,
    /// A weighted mean of the nearest texels.
    Linear,
}

/// What a sampler does with a coordinate outside zero to one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum AddressMode {
    /// The image repeats.
    Repeat,
    /// The edge texel continues.
    ClampToEdge,
}

/// What a sampler is made from.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SamplerDesc {
    /// The filter when one texel covers more than a pixel.
    pub magnify: Filter,
    /// The filter when one texel covers less than a pixel.
    pub minify: Filter,
    /// The filter between mips.
    pub mip: Filter,
    /// The address mode along the width.
    pub address_u: AddressMode,
    /// The address mode along the height.
    pub address_v: AddressMode,
}

/// A sampler, as the index of its entry in the sampler table the backend owns. It is the
/// number a shader indexes the table with, the same on every backend.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct SamplerIndex(pub u32);

/// A slot of the bindless heap. It is the number a shader indexes the heap with, the same
/// on every backend; the caller allocates slots, from zero to the heap's capacity.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct HeapSlot(pub u32);
