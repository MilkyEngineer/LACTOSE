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
/// that calls the constructor. It holds the `Arc` it was given in what destroys the
/// surface and drops it in that destroy, which is the drop of the last of the `Device`,
/// its encoders and its swapchain; it keeps no clone elsewhere, so the window is let go
/// when the device is.
pub trait SurfaceSource: HasWindowHandle + HasDisplayHandle + Send + Sync {}

impl<T: HasWindowHandle + HasDisplayHandle + Send + Sync + ?Sized> SurfaceSource for T {}

/// What a swapchain is made from.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SwapchainDesc {
    /// The extent asked for, with no zero dimension. The surface may give another:
    /// [`Swapchain::extent`] is what it gave.
    pub extent: Extent,
}

/// What an acquire gave.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Acquired {
    /// An image to draw into and present: every subresource `Undefined`, on the present
    /// role's queue. It is a colour attachment and nothing else. The handle is used from
    /// this acquire to its present and at no other time; a later acquire may return it
    /// again or another. A surface that is suboptimal for the swapchain
    /// (`VK_SUBOPTIMAL_KHR`) still gives an image.
    Image(ImageHandle),
    /// No image within the backend's bound. Nothing is wrong: the caller looks at its own
    /// state (a shutdown, a resize) and acquires again.
    None,
    /// The surface no longer matches the swapchain: [`Swapchain::resize`] comes before
    /// the next acquire. On Vulkan, `VK_ERROR_OUT_OF_DATE_KHR`, and
    /// `VK_ERROR_OUT_OF_POOL_MEMORY` after MoltenVK's drawable timeout; never
    /// `VK_SUBOPTIMAL_KHR`. Also returned while the surface's current extent is empty.
    Stale,
}

/// What a present did.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Presented {
    /// The submit was taken and the image queued for display, a suboptimal surface's
    /// (`VK_SUBOPTIMAL_KHR`) included.
    Shown,
    /// The submit was taken, and the surface no longer matches the swapchain: the image
    /// may not have been shown, and [`Swapchain::resize`] comes before the next acquire.
    /// On Vulkan, `VK_ERROR_OUT_OF_DATE_KHR` alone.
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
    /// to bound, by the timeline values its submits signal. Under MoltenVK it returns
    /// within about one second for each image of the swapchain, the drawable wait in its
    /// submit; on other drivers it is bounded by the driver alone, and each device
    /// backend's README names the bound or says there is none.
    ///
    /// # Errors
    ///
    /// `ErrorKind::DeviceLost`; a refusal, as [`Device::submit`](crate::Device::submit)'s,
    /// after which the image is still the acquired one.
    fn present(&mut self, image: ImageHandle, submit: Submit<'_>) -> Result<Presented>;

    /// Makes the swapchain again for the surface as it is now, at about `extent`. The
    /// caller first waits for the points its presents signal; `resize` then waits, within
    /// the bound [`acquire`](Self::acquire) has, for the swapchain's own presents, which
    /// the driver may still read after their submits complete. At a surface whose current
    /// extent is empty it returns without calling the driver and leaves the swapchain
    /// stale; otherwise the images of before are destroyed, and an acquired image is
    /// given up.
    ///
    /// # Errors
    ///
    /// `ErrorKind::DeviceLost`; refused while a present is pending after that bound
    /// ([`Rule::Pending`](crate::Rule::Pending)), which the caller retries; refused for an
    /// extent with a zero dimension ([`Rule::Bounds`](crate::Rule::Bounds)). Both on every
    /// backend.
    fn resize(&mut self, extent: Extent) -> Result<()>;
}
