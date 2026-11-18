//! What a declaration hands back for each thing declared in it.
//!
//! An id is a number in its declaration and means nothing outside it: the handles, which
//! change from frame to frame, stay in the declaration, so that two frames that declare
//! the same things in the same order hold equal ids and compile to one plan (`README.md`,
//! "Declaring").

/// An image of one [`FrameDecl`](crate::FrameDecl): an import or a transient.
///
/// Images are numbered from zero in the order they were declared, imports and transients
/// in one sequence. An id is used with the declaration that issued it, and with what was
/// compiled and recorded from it, and with nothing else.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ImageId(u32);

impl ImageId {
    /// The id of the image declared after `index` others. The declaration's plumbing: a
    /// caller receives its ids from [`FrameDecl`](crate::FrameDecl) and never makes one.
    pub(crate) const fn new(index: u32) -> Self {
        Self(index)
    }

    /// How many images its declaration declared before it.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// A buffer of one [`FrameDecl`](crate::FrameDecl): always an import.
///
/// Buffers are numbered from zero in the order they were imported. An id is used with the
/// declaration that issued it, and with what was compiled and recorded from it, and with
/// nothing else.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BufferId(u32);

impl BufferId {
    /// The id of the buffer imported after `index` others.
    pub(crate) const fn new(index: u32) -> Self {
        Self(index)
    }

    /// How many buffers its declaration imported before it.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// A pass of one [`FrameDecl`](crate::FrameDecl).
///
/// Passes are numbered from zero in the order they were declared, whatever their kind,
/// and that order is the frame's: graph order is the kept passes in it (`README.md`,
/// "Uses and order").
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PassId(u32);

impl PassId {
    /// The id of the pass declared after `index` others.
    pub(crate) const fn new(index: u32) -> Self {
        Self(index)
    }

    /// How many passes its declaration declared before it.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.0
    }
}
