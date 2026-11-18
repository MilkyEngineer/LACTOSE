//! The history ping-pong (architecture section 12.3): two resources of the caller's, of
//! which each frame writes one and reads the one the frame before wrote.

/// Two resources that swap roles every frame: [`current`](Self::current) is the one this
/// frame writes, and [`previous`](Self::previous) the one the last frame wrote.
///
/// `H` is whatever names one of the pair to its owner: an image handle, a buffer handle,
/// or a handle with the heap slot that holds it. The graph does not own a history. Each
/// frame imports both halves, the previous one in the state the last frame left it and
/// the current one from `Undefined`; a handle is not part of a key, so the swap changes
/// no plan. A frame with no previous half declares otherwise and has a key of its own, so
/// a history costs one plan for its first frame and one for every frame after
/// (`README.md`, "History").
///
/// The owner calls [`advance`](Self::advance) once `RenderGraph::submit` has returned the
/// frame's point, and not after a submit that failed: the half that frame was to write
/// holds nothing.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct History<H> {
    /// The two halves.
    pair: [H; 2],
    /// Which half this frame writes: zero or one.
    current: u8,
    /// Whether the other half holds what a frame wrote.
    valid: bool,
}

impl<H: Copy> History<H> {
    /// A history whose first frame writes `pair[0]` and has no previous half.
    #[must_use]
    pub const fn new(pair: [H; 2]) -> Self {
        Self { pair, current: 0, valid: false }
    }

    /// The half this frame writes.
    #[must_use]
    pub const fn current(&self) -> H {
        self.pair[self.current as usize]
    }

    /// The half the last frame wrote, or `None` where no frame has written one since the
    /// history was made or [`reset`](Self::reset).
    #[must_use]
    pub const fn previous(&self) -> Option<H> {
        if self.valid { Some(self.pair[1 - self.current as usize]) } else { None }
    }

    /// Ends a frame that wrote [`current`](Self::current): it becomes
    /// [`previous`](Self::previous), and the other half is the next frame's to write.
    pub const fn advance(&mut self) {
        self.current = 1 - self.current;
        self.valid = true;
    }

    /// Forgets the previous half, as after a resize that remade the pair or a cut the
    /// history must not blend across: [`previous`](Self::previous) is `None` until the
    /// next [`advance`](Self::advance). Which half is current does not change.
    pub const fn reset(&mut self) {
        self.valid = false;
    }
}
