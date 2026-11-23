//! The surface a device presents to: a window's, made from the handles its source lends,
//! or a headless one no window shows (`VK_EXT_headless_surface`), and the instance
//! extensions each needs.

use core::ffi::{CStr, c_void};
use core::ptr::NonNull;
use std::sync::Arc;

use ash::vk;
use lactose_core::{Error, ErrorKind, Result};
use lactose_rhi_api::SurfaceSource;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use crate::raw::{driver, unsupported};

/// What a device is asked to present to.
pub(crate) enum Target {
    /// A surface no window shows.
    Headless,
    /// The window a request names.
    Window(Arc<dyn SurfaceSource>),
}

/// A window's handles, read once each on the constructor's thread.
enum Handles {
    Headless,
    AppKit(NonNull<c_void>),
    Win32 { hinstance: isize, hwnd: isize },
    Xlib { display: *mut c_void, window: core::ffi::c_ulong },
    Xcb { connection: *mut c_void, window: u32 },
    Wayland { display: *mut c_void, surface: *mut c_void },
}

/// A target with its handles read: what the instance and the surface are made from.
pub(crate) struct Chosen {
    handles: Handles,
    source: Option<Arc<dyn SurfaceSource>>,
}

/// Reads the handles of `target`'s window, if it has one: `Unsupported` for a handle that
/// is unavailable or of a platform no surface here is made for.
pub(crate) fn read(target: Target) -> Result<Chosen> {
    let source = match target {
        Target::Headless => return Ok(Chosen { handles: Handles::Headless, source: None }),
        Target::Window(source) => source,
    };
    let unavailable = |what: &str, error: raw_window_handle::HandleError| {
        Error::new(ErrorKind::Unsupported, format!("the window's {what} handle: {error}"))
    };
    let window = source.window_handle().map_err(|e| unavailable("window", e))?.as_raw();
    let display = source.display_handle().map_err(|e| unavailable("display", e))?.as_raw();
    let handles = match (window, display) {
        (RawWindowHandle::AppKit(window), _) => Handles::AppKit(window.ns_view),
        (RawWindowHandle::Win32(window), _) => Handles::Win32 {
            hinstance: window
                .hinstance
                .ok_or_else(|| unsupported("a Win32 window with no instance handle"))?
                .get(),
            hwnd: window.hwnd.get(),
        },
        (RawWindowHandle::Xlib(window), RawDisplayHandle::Xlib(display)) => Handles::Xlib {
            display: display
                .display
                .ok_or_else(|| unsupported("an Xlib window with no display"))?
                .as_ptr(),
            window: window.window,
        },
        (RawWindowHandle::Xcb(window), RawDisplayHandle::Xcb(display)) => Handles::Xcb {
            connection: display
                .connection
                .ok_or_else(|| unsupported("an XCB window with no connection"))?
                .as_ptr(),
            window: window.window.get(),
        },
        (RawWindowHandle::Wayland(window), RawDisplayHandle::Wayland(display)) => {
            Handles::Wayland { display: display.display.as_ptr(), surface: window.surface.as_ptr() }
        }
        (window, display) => {
            return Err(unsupported(format!(
                "a surface for a window of {window:?} on a display of {display:?}"
            )));
        }
    };
    Ok(Chosen { handles, source: Some(source) })
}

impl Chosen {
    /// The instance extension its surface is made through.
    pub(crate) fn extension(&self) -> &'static CStr {
        match self.handles {
            Handles::Headless => ash::ext::headless_surface::NAME,
            Handles::AppKit(_) => ash::ext::metal_surface::NAME,
            Handles::Win32 { .. } => ash::khr::win32_surface::NAME,
            Handles::Xlib { .. } => ash::khr::xlib_surface::NAME,
            Handles::Xcb { .. } => ash::khr::xcb_surface::NAME,
            Handles::Wayland { .. } => ash::khr::wayland_surface::NAME,
        }
    }
}

/// The surface, and what it was made from, kept until it is destroyed.
pub(crate) struct Surface {
    pub(crate) loader: ash::khr::surface::Instance,
    pub(crate) surface: vk::SurfaceKHR,
    /// A window shows it.
    pub(crate) shown: bool,
    /// The window, let go after the surface is destroyed.
    _source: Option<Arc<dyn SurfaceSource>>,
    /// The layer made for an AppKit view, which keeps its bounds and `contentsScale` equal
    /// to the view's on the main thread.
    _layer: Option<Layer>,
}

/// The layer an AppKit view's surface is made on.
#[cfg(target_os = "macos")]
type Layer = raw_window_metal::Layer;
/// No layer is made away from macOS.
#[cfg(not(target_os = "macos"))]
type Layer = core::convert::Infallible;

/// Makes a `CAMetalLayer` for an AppKit view, and the surface on it. The layer is the
/// view's sublayer, kept the view's size and scale on the main thread by
/// `raw-window-metal`'s observer, and its `allowsNextDrawableTimeout` is left true.
///
/// # Safety
///
/// As [`Surface::create`]'s; `view` is an `NSView`.
#[cfg(target_os = "macos")]
unsafe fn metal(
    entry: &ash::Entry,
    instance: &ash::Instance,
    view: NonNull<c_void>,
) -> Result<(vk::SurfaceKHR, Layer)> {
    // `Layer::from_ns_view` panics off the main thread.
    // SAFETY: a query of the calling thread.
    if unsafe { libc::pthread_main_np() } == 0 {
        return Err(unsupported(
            "an AppKit window's surface off the main thread: AppKit's views are the main \
             thread's",
        ));
    }
    // SAFETY: the view is the window's `NSView`, live while its source is.
    let layer = unsafe { raw_window_metal::Layer::from_ns_view(view) };
    let info =
        vk::MetalSurfaceCreateInfoEXT::default().layer(layer.as_ptr().as_ptr().cast_const().cast());
    let metal = ash::ext::metal_surface::Instance::new(entry, instance);
    // SAFETY: the instance has `VK_EXT_metal_surface`; the layer lives while the surface does.
    let surface = unsafe { metal.create_metal_surface(&info, None) }
        .map_err(|e| driver(e, "vkCreateMetalSurfaceEXT"))?;
    Ok((surface, layer))
}

/// An AppKit view has no surface away from macOS.
///
/// # Safety
///
/// None needed; the signature is macOS's.
#[cfg(not(target_os = "macos"))]
unsafe fn metal(
    _: &ash::Entry,
    _: &ash::Instance,
    _: NonNull<c_void>,
) -> Result<(vk::SurfaceKHR, Layer)> {
    Err(unsupported("an AppKit window away from macOS"))
}

impl Surface {
    /// Makes the surface of `chosen` on `instance`, whose extensions include
    /// [`Chosen::extension`] and `VK_KHR_surface`.
    ///
    /// # Safety
    ///
    /// The instance is live, made with those extensions, and outlives the surface; the
    /// handles `chosen` holds are its source's, which it keeps.
    pub(crate) unsafe fn create(
        entry: &ash::Entry,
        instance: &ash::Instance,
        chosen: Chosen,
    ) -> Result<Self> {
        let loader = ash::khr::surface::Instance::new(entry, instance);
        let mut layer = None;
        // In each arm, the instance has the arm's extension, and each handle is one the
        // window's source lent, live while the source is, which the surface keeps.
        let made = match chosen.handles {
            Handles::Headless => {
                let info = vk::HeadlessSurfaceCreateInfoEXT::default();
                let headless = ash::ext::headless_surface::Instance::new(entry, instance);
                // SAFETY: as the arms' comment above.
                unsafe { headless.create_headless_surface(&info, None) }
            }
            Handles::AppKit(view) => {
                // SAFETY: as the arms' comment above; the view is an `NSView`.
                let (surface, made) = unsafe { metal(entry, instance, view) }?;
                layer = Some(made);
                Ok(surface)
            }
            Handles::Win32 { hinstance, hwnd } => {
                let info = vk::Win32SurfaceCreateInfoKHR::default().hinstance(hinstance).hwnd(hwnd);
                let win32 = ash::khr::win32_surface::Instance::new(entry, instance);
                // SAFETY: as the arms' comment above.
                unsafe { win32.create_win32_surface(&info, None) }
            }
            Handles::Xlib { display, window } => {
                let info =
                    vk::XlibSurfaceCreateInfoKHR::default().dpy(display.cast()).window(window);
                let xlib = ash::khr::xlib_surface::Instance::new(entry, instance);
                // SAFETY: as the arms' comment above.
                unsafe { xlib.create_xlib_surface(&info, None) }
            }
            Handles::Xcb { connection, window } => {
                let info = vk::XcbSurfaceCreateInfoKHR::default()
                    .connection(connection.cast())
                    .window(window);
                let xcb = ash::khr::xcb_surface::Instance::new(entry, instance);
                // SAFETY: as the arms' comment above.
                unsafe { xcb.create_xcb_surface(&info, None) }
            }
            Handles::Wayland { display, surface } => {
                let info = vk::WaylandSurfaceCreateInfoKHR::default()
                    .display(display.cast())
                    .surface(surface.cast());
                let wayland = ash::khr::wayland_surface::Instance::new(entry, instance);
                // SAFETY: as the arms' comment above.
                unsafe { wayland.create_wayland_surface(&info, None) }
            }
        };
        let surface = made.map_err(|e| driver(e, "vkCreate*SurfaceKHR"))?;
        Ok(Self {
            loader,
            surface,
            shown: !matches!(chosen.handles, Handles::Headless),
            _source: chosen.source,
            _layer: layer,
        })
    }

    /// Destroys the surface.
    ///
    /// # Safety
    ///
    /// No swapchain of it lives, its instance does, and it is destroyed once.
    pub(crate) unsafe fn destroy(&self) {
        // SAFETY: by the caller's promise.
        unsafe { self.loader.destroy_surface(self.surface, None) };
    }

    /// Whether `family` of `physical` can present to it.
    pub(crate) fn supports(&self, physical: vk::PhysicalDevice, family: u32) -> Result<bool> {
        // SAFETY: the surface and the physical device are of one live instance.
        unsafe { self.loader.get_physical_device_surface_support(physical, family, self.surface) }
            .map_err(|e| driver(e, "vkGetPhysicalDeviceSurfaceSupportKHR"))
    }
}
