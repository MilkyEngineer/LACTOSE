//! The surface and the swapchain: where a window comes in, and what the present thread
//! holds.
//!
//! A surface is made from `raw-window-handle`'s types, so that a backend names the
//! windowing crate's handles and never `lactose-platform` (`README.md`, "The surface").

use lactose_core::Result;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use crate::resource::{Extent, Format, ImageHandle};
use crate::sync::Submit;

/// A window a device can present to: anything that lends `raw-window-handle`'s window
/// and display handles and may be shared between threads. Implemented for every such
/// type; `lactose-platform`'s window is one.
///
/// A backend calls [`HasWindowHandle::window_handle`] and
/// [`HasDisplayHandle::display_handle`] once each, inside its constructor, on the thread
/// that calls the constructor, and keeps the `Arc` it was given until its surface is
/// gone, so the window outlives every use of its handles.
pub trait SurfaceSource: HasWindowHandle + HasDisplayHandle + Send + Sync {}

impl<T: HasWindowHandle + HasDisplayHandle + Send + Sync + ?Sized> SurfaceSource for T {}

/// What a swapchain is made from.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SwapchainDesc {
    /// The extent asked for. The surface may give another:
    /// [`Swapchain::extent`] is what it gave.
    pub extent: Extent,
}

/// What an acquire gave.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Acquired {
    /// An image to draw into and present: every subresource `Undefined`, on the present
    /// role's queue. It is a colour attachment and nothing else. The handle is used from
    /// this acquire to its present and at no other time; a later acquire may return it
    /// again or another.
    Image(ImageHandle),
    /// No image within the backend's bound. Nothing is wrong: the caller looks at its own
    /// state (a shutdown, a resize) and acquires again.
    None,
    /// The surface no longer matches the swapchain: [`Swapchain::resize`] comes before
    /// the next acquire.
    Stale,
}

/// What a present did.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Presented {
    /// The submit was taken and the image queued for display.
    Shown,
    /// The submit was taken, and the surface no longer matches the swapchain: the image
    /// may not have been shown, and [`Swapchain::resize`] comes before the next acquire.
    Stale,
}

/// A device's swapchain, which one thread holds: the present thread.
///
/// Every command that reads or writes a swapchain image is recorded on an encoder of
/// [`QueueRole::Present`](crate::QueueRole::Present) and submitted through
/// [`present`](Self::present), which is the only way a submit reaches the present role.
/// Where the device has a present queue of its own no other thread's submit waits behind
/// it; where it has not, `present` takes the graphics queue's lock like any submit, and
/// the caller's code is the same (`README.md`, "The present thread").
pub trait Swapchain: Send {
    /// The format of the swapchain's images: the surface's. The present thread's pipeline
    /// is made for it.
    fn format(&self) -> Format;

    /// The extent of the swapchain's images.
    fn extent(&self) -> Extent;

    /// How many images the swapchain cycles through.
    fn image_count(&self) -> u32;

    /// Takes the next image. It returns within a bound the backend sets, about a second,
    /// and takes no timeout: with no image by then it returns [`Acquired::None`]. One
    /// image is held at a time: a second acquire before the first image is presented is
    /// refused ([`Rule::Swapchain`](crate::Rule::Swapchain)).
    ///
    /// # Errors
    ///
    /// `ErrorKind::DeviceLost`.
    fn acquire(&mut self) -> Result<Acquired>;

    /// Submits `submit` on the present role's queue and presents `image`, the acquired
    /// one, which the submit leaves in [`ImageState::Present`](crate::ImageState). The
    /// submit's encoders are of the present role. It returns once the submit is taken,
    /// not once the image is on screen; how many presents are in flight is the caller's
    /// to bound, by the timeline values its submits signal.
    ///
    /// # Errors
    ///
    /// `ErrorKind::DeviceLost`; a refusal, as [`Device::submit`](crate::Device::submit)'s,
    /// after which the image is still the acquired one.
    fn present(&mut self, image: ImageHandle, submit: Submit<'_>) -> Result<Presented>;

    /// Makes the swapchain again for the surface as it is now, at about `extent`. The
    /// caller first waits for every present it made; the images of before are destroyed,
    /// and an acquired image is given up.
    ///
    /// # Errors
    ///
    /// `ErrorKind::DeviceLost`; refused while a present is pending
    /// ([`Rule::Pending`](crate::Rule::Pending)).
    fn resize(&mut self, extent: Extent) -> Result<()>;
}
