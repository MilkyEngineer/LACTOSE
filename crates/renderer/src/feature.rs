//! The pass-provider trait (architecture section 12.5): how `lactose-terrain`,
//! `lactose-debug-ui` and `lactose-ui` add passes to the frame the renderer declares.
//! Milestone 2 has no provider; the trait is here so that the frame's shape does not
//! change when the first arrives (`README.md`, "Features").

use lactose_render_graph::{BufferId, FrameDecl, ImageId};
use lactose_rhi_api::Extent;

/// What a provider's passes may use of the frame the renderer declared, by the ids of the
/// declaration it is handed.
///
/// A later milestone adds what its passes introduce (the heap's slots, the G-buffer), so
/// the type is not exhaustive: a provider reads its fields and never builds one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
pub struct FeatureView {
    /// The frame's colour target, as the triangle pass left it. A provider's pass draws
    /// over it with `Load::Keep`.
    pub target: ImageId,
    /// The depth the triangle pass wrote, a `Depth32Float` image of the target's extent:
    /// zero at the near plane, one at the far plane, compared with `Compare::Less`.
    pub depth: ImageId,
    /// The frame's camera buffer, which holds a [`CameraData`](crate::CameraData) by the
    /// time the frame is submitted and nothing a recorder may read before then. A pass
    /// declares it with `BufferAccess::Read` and pushes its address.
    pub camera: BufferId,
    /// The target's extent.
    pub extent: Extent,
}

/// A provider of passes: a crate's part of the frame, registered once with
/// [`Renderer::add_feature`](crate::Renderer::add_feature) by the client binary.
///
/// `Send + Sync`, because the recorders of the passes it declares borrow it and run on
/// the recording jobs, several at once.
pub trait RenderFeature: Send + Sync {
    /// The provider's name, for an error or a log line that names it.
    fn label(&self) -> &str;

    /// Declares the provider's passes for one frame into `frame`, after the renderer's
    /// own and after those of every provider registered before it.
    ///
    /// The provider imports what is its own, declares its passes with their recorders,
    /// and returns. It makes its pipelines when it is made and not here, reaches no
    /// device, takes no lock and waits for nothing: the call is on the frame's path. A
    /// recorder may borrow the provider for `'f`, which ends when the frame is recorded.
    /// What `frame` refuses, it refuses at `RenderGraph::compile`, naming the pass by its
    /// label.
    fn declare<'f>(&'f self, frame: &mut FrameDecl<'f>, view: &FeatureView);
}
