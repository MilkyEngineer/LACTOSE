//! The facade's values as Vulkan's: formats, usages, the scope of each state, and the
//! barrier a transition records on the queue that records it (`lactose-rhi-api`'s README,
//! "States"). Nothing here calls the driver.

use ash::vk;
use lactose_rhi_api::{
    AddressMode, BufferState, BufferUsage, Compare, Cull, Filter, Format, ImageState, ImageUsage,
};

pub(crate) fn format(format: Format) -> vk::Format {
    match format {
        Format::Rgba8Unorm => vk::Format::R8G8B8A8_UNORM,
        Format::Rgba8Srgb => vk::Format::R8G8B8A8_SRGB,
        Format::Bgra8Unorm => vk::Format::B8G8R8A8_UNORM,
        Format::Bgra8Srgb => vk::Format::B8G8R8A8_SRGB,
        Format::Rgba16Float => vk::Format::R16G16B16A16_SFLOAT,
        Format::R32Uint => vk::Format::R32_UINT,
        Format::R32Float => vk::Format::R32_SFLOAT,
        Format::Depth32Float => vk::Format::D32_SFLOAT,
        // A format of a later contract: no driver call is made with it (`check_image`,
        // `pipeline::graphics`).
        _ => vk::Format::UNDEFINED,
    }
}

pub(crate) fn aspect(format: Format) -> vk::ImageAspectFlags {
    if format.is_depth() { vk::ImageAspectFlags::DEPTH } else { vk::ImageAspectFlags::COLOR }
}

pub(crate) fn image_usage(usage: ImageUsage) -> vk::ImageUsageFlags {
    let pairs = [
        (ImageUsage::SAMPLED, vk::ImageUsageFlags::SAMPLED),
        (ImageUsage::COLOUR, vk::ImageUsageFlags::COLOR_ATTACHMENT),
        (ImageUsage::DEPTH, vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT),
        (ImageUsage::INPUT, vk::ImageUsageFlags::INPUT_ATTACHMENT),
        (ImageUsage::TRANSIENT, vk::ImageUsageFlags::TRANSIENT_ATTACHMENT),
        (ImageUsage::COPY_SOURCE, vk::ImageUsageFlags::TRANSFER_SRC),
        (ImageUsage::COPY_DESTINATION, vk::ImageUsageFlags::TRANSFER_DST),
    ];
    pairs
        .iter()
        .filter(|(ours, _)| usage.contains(*ours))
        .fold(vk::ImageUsageFlags::empty(), |all, (_, theirs)| all | *theirs)
}

/// A buffer's usage. A buffer with none the facade names is made a copy source, which no
/// command reaches without the facade's flag: Vulkan has no buffer of no usage.
pub(crate) fn buffer_usage(usage: BufferUsage) -> vk::BufferUsageFlags {
    let pairs = [
        (BufferUsage::DEVICE_ADDRESS, vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS),
        (BufferUsage::INDEX, vk::BufferUsageFlags::INDEX_BUFFER),
        (BufferUsage::INDIRECT, vk::BufferUsageFlags::INDIRECT_BUFFER),
        (BufferUsage::COPY_SOURCE, vk::BufferUsageFlags::TRANSFER_SRC),
        (BufferUsage::COPY_DESTINATION, vk::BufferUsageFlags::TRANSFER_DST),
    ];
    let flags = pairs
        .iter()
        .filter(|(ours, _)| usage.contains(*ours))
        .fold(vk::BufferUsageFlags::empty(), |all, (_, theirs)| all | *theirs);
    if flags.is_empty() { vk::BufferUsageFlags::TRANSFER_SRC } else { flags }
}

pub(crate) fn compare(compare: Compare) -> vk::CompareOp {
    match compare {
        Compare::Never => vk::CompareOp::NEVER,
        Compare::Less => vk::CompareOp::LESS,
        Compare::LessEqual => vk::CompareOp::LESS_OR_EQUAL,
        Compare::Equal => vk::CompareOp::EQUAL,
        Compare::GreaterEqual => vk::CompareOp::GREATER_OR_EQUAL,
        Compare::Greater => vk::CompareOp::GREATER,
        Compare::Always => vk::CompareOp::ALWAYS,
    }
}

pub(crate) fn cull(cull: Cull) -> vk::CullModeFlags {
    match cull {
        Cull::None => vk::CullModeFlags::NONE,
        Cull::Front => vk::CullModeFlags::FRONT,
        Cull::Back => vk::CullModeFlags::BACK,
    }
}

pub(crate) fn filter(filter: Filter) -> vk::Filter {
    match filter {
        Filter::Nearest => vk::Filter::NEAREST,
        Filter::Linear => vk::Filter::LINEAR,
    }
}

pub(crate) fn mip_filter(filter: Filter) -> vk::SamplerMipmapMode {
    match filter {
        Filter::Nearest => vk::SamplerMipmapMode::NEAREST,
        Filter::Linear => vk::SamplerMipmapMode::LINEAR,
    }
}

pub(crate) fn address(mode: AddressMode) -> vk::SamplerAddressMode {
    match mode {
        AddressMode::Repeat => vk::SamplerAddressMode::REPEAT,
        AddressMode::ClampToEdge => vk::SamplerAddressMode::CLAMP_TO_EDGE,
    }
}

/// What a state's uses are to a barrier: the stages that use it, every access they make,
/// the accesses among them that write, and for an image its layout.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Scope {
    pub(crate) stages: vk::PipelineStageFlags2,
    pub(crate) access: vk::AccessFlags2,
    pub(crate) writes: vk::AccessFlags2,
    pub(crate) layout: vk::ImageLayout,
}

type S = vk::PipelineStageFlags2;
type A = vk::AccessFlags2;

/// The stages a shader in any pipeline runs in.
const SHADERS: S = S::from_raw(
    S::VERTEX_SHADER.as_raw() | S::FRAGMENT_SHADER.as_raw() | S::COMPUTE_SHADER.as_raw(),
);

/// The stages and accesses no queue without `GRAPHICS` has (verification row 214).
const GRAPHICS_STAGES: S = S::from_raw(
    S::VERTEX_SHADER.as_raw()
        | S::FRAGMENT_SHADER.as_raw()
        | S::COLOR_ATTACHMENT_OUTPUT.as_raw()
        | S::EARLY_FRAGMENT_TESTS.as_raw()
        | S::LATE_FRAGMENT_TESTS.as_raw()
        | S::INDEX_INPUT.as_raw()
        | S::VERTEX_INPUT.as_raw()
        | S::VERTEX_ATTRIBUTE_INPUT.as_raw(),
);
const GRAPHICS_ACCESS: A = A::from_raw(
    A::COLOR_ATTACHMENT_READ.as_raw()
        | A::COLOR_ATTACHMENT_WRITE.as_raw()
        | A::DEPTH_STENCIL_ATTACHMENT_READ.as_raw()
        | A::DEPTH_STENCIL_ATTACHMENT_WRITE.as_raw()
        | A::INPUT_ATTACHMENT_READ.as_raw()
        | A::INDEX_READ.as_raw()
        | A::VERTEX_ATTRIBUTE_READ.as_raw(),
);

const fn scope(stages: S, access: A, writes: A, layout: vk::ImageLayout) -> Scope {
    Scope { stages, access, writes, layout }
}

pub(crate) const fn image_scope(state: ImageState) -> Scope {
    use vk::ImageLayout as L;
    match state {
        ImageState::Undefined => scope(S::NONE, A::NONE, A::NONE, L::UNDEFINED),
        ImageState::ColourAttachment => scope(
            S::COLOR_ATTACHMENT_OUTPUT,
            A::from_raw(A::COLOR_ATTACHMENT_READ.as_raw() | A::COLOR_ATTACHMENT_WRITE.as_raw()),
            A::COLOR_ATTACHMENT_WRITE,
            L::COLOR_ATTACHMENT_OPTIMAL,
        ),
        ImageState::LocalRead => scope(
            S::from_raw(S::COLOR_ATTACHMENT_OUTPUT.as_raw() | S::FRAGMENT_SHADER.as_raw()),
            A::from_raw(
                A::COLOR_ATTACHMENT_READ.as_raw()
                    | A::COLOR_ATTACHMENT_WRITE.as_raw()
                    | A::INPUT_ATTACHMENT_READ.as_raw(),
            ),
            A::COLOR_ATTACHMENT_WRITE,
            L::RENDERING_LOCAL_READ_KHR,
        ),
        ImageState::DepthAttachment => scope(
            S::from_raw(S::EARLY_FRAGMENT_TESTS.as_raw() | S::LATE_FRAGMENT_TESTS.as_raw()),
            A::from_raw(
                A::DEPTH_STENCIL_ATTACHMENT_READ.as_raw()
                    | A::DEPTH_STENCIL_ATTACHMENT_WRITE.as_raw(),
            ),
            A::DEPTH_STENCIL_ATTACHMENT_WRITE,
            L::DEPTH_ATTACHMENT_OPTIMAL,
        ),
        ImageState::SampledRead => {
            scope(SHADERS, A::SHADER_SAMPLED_READ, A::NONE, L::SHADER_READ_ONLY_OPTIMAL)
        }
        ImageState::CopySource => {
            scope(S::COPY, A::TRANSFER_READ, A::NONE, L::TRANSFER_SRC_OPTIMAL)
        }
        ImageState::CopyDestination => {
            scope(S::COPY, A::TRANSFER_WRITE, A::TRANSFER_WRITE, L::TRANSFER_DST_OPTIMAL)
        }
        // `PRESENT_SRC_KHR` needs the swapchain's extension: the encoder gives a swapchain's
        // image that layout, and any other image moved to `Present` is in this one.
        ImageState::Present => scope(S::ALL_COMMANDS, A::NONE, A::NONE, L::GENERAL),
    }
}

pub(crate) const fn buffer_scope(state: BufferState) -> Scope {
    let layout = vk::ImageLayout::UNDEFINED;
    match state {
        BufferState::Undefined => scope(S::NONE, A::NONE, A::NONE, layout),
        BufferState::ShaderRead => scope(SHADERS, A::SHADER_STORAGE_READ, A::NONE, layout),
        BufferState::ShaderWrite => scope(
            SHADERS,
            A::from_raw(A::SHADER_STORAGE_READ.as_raw() | A::SHADER_STORAGE_WRITE.as_raw()),
            A::SHADER_STORAGE_WRITE,
            layout,
        ),
        BufferState::Index => scope(S::INDEX_INPUT, A::INDEX_READ, A::NONE, layout),
        BufferState::Indirect => scope(S::DRAW_INDIRECT, A::INDIRECT_COMMAND_READ, A::NONE, layout),
        BufferState::CopySource => scope(S::COPY, A::TRANSFER_READ, A::NONE, layout),
        BufferState::CopyDestination => {
            scope(S::COPY, A::TRANSFER_WRITE, A::TRANSFER_WRITE, layout)
        }
        BufferState::HostRead => scope(S::HOST, A::HOST_READ, A::NONE, layout),
    }
}

/// A queue as a barrier sees it: its index among the device's queues, its family, and
/// whether the family draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Side {
    pub(crate) queue: usize,
    pub(crate) family: u32,
    pub(crate) graphics: bool,
}

/// One barrier, as a buffer's or an image's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Barrier {
    pub(crate) src_stages: S,
    pub(crate) src_access: A,
    pub(crate) dst_stages: S,
    pub(crate) dst_access: A,
    pub(crate) old_layout: vk::ImageLayout,
    pub(crate) new_layout: vk::ImageLayout,
    /// The source and destination families of an ownership transfer.
    pub(crate) families: Option<(u32, u32)>,
}

/// One change of state, as a transition gives it: its two scopes, whether its `from` is
/// `Undefined`, and the queues its two roles resolve to.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Change {
    pub(crate) from: Scope,
    pub(crate) to: Scope,
    pub(crate) from_undefined: bool,
    pub(crate) from_queue: Side,
    pub(crate) to_queue: Side,
    /// Made `Sharing::Exclusive`: owned by one queue's family at a time.
    pub(crate) exclusive: bool,
}

/// Holds a scope to what a family has: no family without `GRAPHICS` names a graphics
/// stage (verification row 214). A scope left with no stage takes every command, so the
/// dependency it stood for is kept.
fn masked(stages: S, access: A, graphics: bool, everything: A) -> (S, A) {
    if graphics {
        return (stages, access);
    }
    let kept = stages & !GRAPHICS_STAGES;
    if kept.is_empty() && !stages.is_empty() {
        return (S::ALL_COMMANDS, everything);
    }
    (kept, access & !GRAPHICS_ACCESS)
}

/// The families of an ownership transfer between two families that `change` is, if it is
/// one.
fn families(change: &Change) -> Option<(u32, u32)> {
    let (from, to) = (change.from_queue, change.to_queue);
    let crossing = change.exclusive && !change.from_undefined && from.queue != to.queue;
    (crossing && from.family != to.family).then_some((from.family, to.family))
}

/// `stages` and `access` with the host's stage and accesses taken out, `instead` put in the
/// host stage's place.
fn off_host(stages: S, access: A, instead: S) -> (S, A) {
    if !stages.contains(S::HOST) {
        return (stages, access);
    }
    ((stages & !S::HOST) | instead, access & !(A::HOST_READ | A::HOST_WRITE))
}

/// The barrier `change` records on the queue `recording`, if any.
///
/// A change from `Undefined` takes every earlier command and write as its source and
/// discards the contents (verification row 211). A change of a shared resource, or one
/// whose two roles one queue serves, is one barrier. An ownership transfer between two
/// queues of one family is the full change on the releasing queue and nothing on the
/// acquiring one, which the semaphore between them orders (D160, row 213); between two
/// families it is a release and an acquire of equal layouts and families, each holding its
/// own half of the scope. Neither half names the host, which a barrier between two families
/// may not (`VUID-VkBufferMemoryBarrier2-srcStageMask-03851`): a release out of `HostRead`
/// takes no source stage for it, since the submit orders the host's earlier reads, and an
/// acquire into `HostRead` takes every command as its destination, the host's half being
/// [`host_after`]'s barrier.
pub(crate) fn barrier(change: &Change, recording: Side) -> Option<Barrier> {
    let src = if change.from_undefined {
        (S::ALL_COMMANDS, A::MEMORY_WRITE)
    } else {
        (change.from.stages, change.from.writes)
    };
    let dst = (change.to.stages, change.to.access);
    let full = |side: Side| {
        let (src_stages, src_access) = masked(src.0, src.1, side.graphics, A::MEMORY_WRITE);
        let (dst_stages, dst_access) =
            masked(dst.0, dst.1, side.graphics, A::MEMORY_READ | A::MEMORY_WRITE);
        Barrier {
            src_stages,
            src_access,
            dst_stages,
            dst_access,
            old_layout: if change.from_undefined {
                vk::ImageLayout::UNDEFINED
            } else {
                change.from.layout
            },
            new_layout: change.to.layout,
            families: None,
        }
    };
    let (from, to) = (change.from_queue, change.to_queue);
    let crossing = change.exclusive && !change.from_undefined && from.queue != to.queue;
    if !crossing {
        return Some(full(recording));
    }
    let Some(pair) = families(change) else {
        return if recording.queue == to.queue { None } else { Some(full(recording)) };
    };
    let families = Some(pair);
    if recording.queue == from.queue {
        let release = full(from);
        let (src_stages, src_access) = off_host(release.src_stages, release.src_access, S::NONE);
        Some(Barrier {
            src_stages,
            src_access,
            dst_stages: S::NONE,
            dst_access: A::NONE,
            families,
            ..release
        })
    } else if recording.queue == to.queue {
        let acquire = full(to);
        let (dst_stages, dst_access) =
            off_host(acquire.dst_stages, acquire.dst_access, S::ALL_COMMANDS);
        Some(Barrier {
            src_stages: S::NONE,
            src_access: A::NONE,
            dst_stages,
            dst_access,
            families,
            ..acquire
        })
    } else {
        Some(full(recording))
    }
}

/// The barrier the queue `recording` records after `change`'s acquire, in a later call, when
/// the acquire is one between two families into a scope of the host's: on the acquiring
/// family alone, it makes every earlier write visible to the host, the acquired ones
/// included.
pub(crate) fn host_after(change: &Change, recording: Side) -> Option<Barrier> {
    let into_host = change.to.stages.contains(S::HOST);
    if !into_host || recording.queue != change.to_queue.queue || families(change).is_none() {
        return None;
    }
    Some(Barrier {
        src_stages: S::ALL_COMMANDS,
        src_access: A::MEMORY_WRITE,
        dst_stages: S::HOST,
        dst_access: change.to.access & (A::HOST_READ | A::HOST_WRITE),
        old_layout: change.to.layout,
        new_layout: change.to.layout,
        families: None,
    })
}

#[cfg(test)]
mod tests {
    use ash::vk;
    use lactose_rhi_api::{BufferState, ImageState};

    use super::{Barrier, Change, Side, barrier, buffer_scope, host_after, image_scope};

    const GRAPHICS: Side = Side { queue: 0, family: 0, graphics: true };
    /// A second queue of the graphics queue's family.
    const SIBLING: Side = Side { queue: 1, family: 0, graphics: true };
    /// A queue of a family that computes and does not draw.
    const COMPUTE: Side = Side { queue: 1, family: 2, graphics: false };

    fn change(from: ImageState, to: ImageState, from_queue: Side, to_queue: Side) -> Change {
        Change {
            from: image_scope(from),
            to: image_scope(to),
            from_undefined: from == ImageState::Undefined,
            from_queue,
            to_queue,
            exclusive: true,
        }
    }

    #[test]
    fn retake_from_undefined_waits_for_every_earlier_write() {
        let made = change(ImageState::Undefined, ImageState::ColourAttachment, GRAPHICS, GRAPHICS);
        let b = barrier(&made, GRAPHICS).unwrap();
        assert_eq!(
            (b.src_stages, b.src_access),
            (vk::PipelineStageFlags2::ALL_COMMANDS, vk::AccessFlags2::MEMORY_WRITE)
        );
        assert_eq!(b.old_layout, vk::ImageLayout::UNDEFINED);
        assert_eq!(b.families, None);
        // From `Undefined` across queues: one barrier, on the queue that records it, no
        // transfer.
        let across = change(ImageState::Undefined, ImageState::SampledRead, GRAPHICS, COMPUTE);
        assert_eq!(barrier(&across, COMPUTE).unwrap().families, None);
    }

    #[test]
    fn same_family_transfer_is_one_layout_change_on_the_releasing_queue() {
        let made = change(ImageState::ColourAttachment, ImageState::SampledRead, GRAPHICS, SIBLING);
        let release = barrier(&made, GRAPHICS).unwrap();
        assert_eq!(release.families, None);
        assert_eq!(
            (release.old_layout, release.new_layout),
            (vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        );
        assert_eq!(release.dst_access, vk::AccessFlags2::SHADER_SAMPLED_READ);
        assert_eq!(barrier(&made, SIBLING), None);
    }

    #[test]
    fn transfer_between_families_is_a_release_and_an_acquire_of_equal_values() {
        let made = change(ImageState::ColourAttachment, ImageState::SampledRead, GRAPHICS, COMPUTE);
        let release = barrier(&made, GRAPHICS).unwrap();
        let acquire = barrier(&made, COMPUTE).unwrap();
        for half in [release, acquire] {
            assert_eq!(half.families, Some((0, 2)));
            assert_eq!(half.old_layout, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
            assert_eq!(half.new_layout, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        }
        assert_eq!(
            (release.dst_stages, release.dst_access),
            (vk::PipelineStageFlags2::NONE, vk::AccessFlags2::NONE)
        );
        assert_eq!(
            (acquire.src_stages, acquire.src_access),
            (vk::PipelineStageFlags2::NONE, vk::AccessFlags2::NONE)
        );
        assert_eq!(release.src_access, vk::AccessFlags2::COLOR_ATTACHMENT_WRITE);
    }

    fn buffer_change(
        from: BufferState,
        to: BufferState,
        from_queue: Side,
        to_queue: Side,
    ) -> Change {
        Change {
            from: buffer_scope(from),
            to: buffer_scope(to),
            from_undefined: false,
            from_queue,
            to_queue,
            exclusive: true,
        }
    }

    #[test]
    fn no_barrier_between_families_names_the_host() {
        let host = vk::PipelineStageFlags2::HOST;
        let host_access = vk::AccessFlags2::HOST_READ | vk::AccessFlags2::HOST_WRITE;
        let named = |b: Barrier| {
            b.src_stages.intersects(host)
                || b.dst_stages.intersects(host)
                || b.src_access.intersects(host_access)
                || b.dst_access.intersects(host_access)
        };
        // Into `HostRead`: the acquire takes every command, and the host's barrier after it,
        // on the acquiring family alone, takes the writes to the host.
        let into =
            buffer_change(BufferState::ShaderWrite, BufferState::HostRead, COMPUTE, GRAPHICS);
        let release = barrier(&into, COMPUTE).unwrap();
        let acquire = barrier(&into, GRAPHICS).unwrap();
        assert!(!named(release) && !named(acquire));
        assert_eq!(acquire.families, Some((2, 0)));
        assert_eq!(acquire.dst_stages, vk::PipelineStageFlags2::ALL_COMMANDS);
        assert_eq!(host_after(&into, COMPUTE), None);
        let after = host_after(&into, GRAPHICS).unwrap();
        assert_eq!(after.families, None);
        assert_eq!(
            (after.src_stages, after.src_access, after.dst_stages, after.dst_access),
            (
                vk::PipelineStageFlags2::ALL_COMMANDS,
                vk::AccessFlags2::MEMORY_WRITE,
                host,
                vk::AccessFlags2::HOST_READ
            )
        );
        // Out of `HostRead`: the release takes no source stage, the submit ordering the
        // host's earlier read, and the acquire is the use's.
        let out = buffer_change(BufferState::HostRead, BufferState::CopySource, COMPUTE, GRAPHICS);
        let release = barrier(&out, COMPUTE).unwrap();
        assert!(!named(release) && !named(barrier(&out, GRAPHICS).unwrap()));
        assert_eq!(release.src_stages, vk::PipelineStageFlags2::NONE);
        assert_eq!(host_after(&out, GRAPHICS), None);
        // Within one queue, or between two queues of one family, the host is named as before.
        let one =
            buffer_change(BufferState::CopyDestination, BufferState::HostRead, GRAPHICS, GRAPHICS);
        assert_eq!(barrier(&one, GRAPHICS).unwrap().dst_stages, host);
        assert_eq!(host_after(&one, GRAPHICS), None);
        let sibling =
            buffer_change(BufferState::CopyDestination, BufferState::HostRead, GRAPHICS, SIBLING);
        assert_eq!(barrier(&sibling, GRAPHICS).unwrap().dst_stages, host);
        assert_eq!(host_after(&sibling, SIBLING), None);
    }

    #[test]
    fn a_family_without_graphics_records_no_graphics_stage() {
        let made = change(ImageState::ColourAttachment, ImageState::SampledRead, GRAPHICS, COMPUTE);
        let Barrier { dst_stages, dst_access, .. } = barrier(&made, COMPUTE).unwrap();
        assert_eq!(dst_stages, vk::PipelineStageFlags2::COMPUTE_SHADER);
        assert_eq!(dst_access, vk::AccessFlags2::SHADER_SAMPLED_READ);
        // A buffer's index use, which only a graphics queue has, taken on a compute queue
        // as a plain change: every command, never a graphics stage.
        let index = Change {
            from: buffer_scope(BufferState::CopyDestination),
            to: buffer_scope(BufferState::Index),
            from_undefined: false,
            from_queue: COMPUTE,
            to_queue: COMPUTE,
            exclusive: true,
        };
        let b = barrier(&index, COMPUTE).unwrap();
        assert_eq!(b.dst_stages, vk::PipelineStageFlags2::ALL_COMMANDS);
        assert!(!b.src_stages.intersects(vk::PipelineStageFlags2::INDEX_INPUT));
    }

    #[test]
    fn same_state_in_a_writing_state_orders_the_writes() {
        let made =
            change(ImageState::ColourAttachment, ImageState::ColourAttachment, GRAPHICS, GRAPHICS);
        let b = barrier(&made, GRAPHICS).unwrap();
        assert_eq!(b.src_access, vk::AccessFlags2::COLOR_ATTACHMENT_WRITE);
        assert!(b.dst_access.contains(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE));
    }
}
