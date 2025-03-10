//! Utilities for generic window handling

use std::ffi::CStr;

use ash::{extensions::khr, vk};
use raw_window_handle::{
    HandleError, HasRawDisplayHandle, HasRawWindowHandle, RawDisplayHandle, RawWindowHandle
};
#[cfg(feature = "winit")]
use winit;

/// Trait for windows that exposes the content width and height of a window.
pub trait WindowSize {
    /// Get the width of the window
    fn width(&self) -> u32;
    /// Get the height of the window
    fn height(&self) -> u32;
}

/// Used as a dummy window interface in case of a headless context. Calling any of the `raw_xxx_handle()` functions on this will result in a panic.
pub struct HeadlessWindowInterface;

unsafe impl HasRawWindowHandle for HeadlessWindowInterface {
    fn raw_window_handle(&self) -> Result<RawWindowHandle, HandleError> {
        panic!("Called raw_window_handle() on headless window context.");
    }
}

unsafe impl HasRawDisplayHandle for HeadlessWindowInterface {
    fn raw_display_handle(&self) -> Result<RawDisplayHandle, HandleError> {
        panic!("Called raw_display_handle() on headless window context.");
    }
}

impl WindowSize for HeadlessWindowInterface {
    fn width(&self) -> u32 {
        panic!("called width() on headless window context.");
    }

    fn height(&self) -> u32 {
        panic!("Called height() on headless window context");
    }
}

#[cfg(feature = "winit")]
impl WindowSize for winit::window::Window {
    fn width(&self) -> u32 {
        self.inner_size().width
    }

    fn height(&self) -> u32 {
        self.inner_size().height
    }
}

/// Blanket trait combining all requirements for a window interface. To be a window interface, a type T must implement the following traits:
/// - [`HasRawWindowHandle`](raw_window_handle::HasRawWindowHandle)
/// - [`HasRawDisplayHandle`](raw_window_handle::HasRawDisplayHandle)
/// - [`WindowSize`]
pub trait WindowInterface: HasRawWindowHandle + HasRawDisplayHandle + WindowSize {
    fn vk_wsi_exts(&self) -> Vec<&'static CStr> {
        match self.raw_window_handle().unwrap() {
            #[cfg(target_os = "windows")]
            RawWindowHandle::Windows(_) => vec![khr::Surface::name(), khr::Win32Surface::name()],

            #[cfg(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ))]
            RawWindowHandle::Wayland(_) => vec![khr::Surface::name(), khr::WaylandSurface::name()],

            #[cfg(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ))]
            RawWindowHandle::Xlib(_) => vec![khr::Surface::name(), khr::XlibSurface::name()],

            #[cfg(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ))]
            RawWindowHandle::Xcb(_) => vec![khr::Surface::name(), khr::XcbSurface::name()],

            #[cfg(any(target_os = "android"))]
            RawWindowHandle::Android(_) => vec![khr::Surface::name(), khr::AndroidSurface::name()],

            #[cfg(any(target_os = "macos"))]
            RawWindowHandle::MacOS(_) => vec![khr::Surface::name(), ext::MetalSurface::name()],

            #[cfg(any(target_os = "ios"))]
            RawWindowHandle::IOS(_) => vec![khr::Surface::name(), ext::MetalSurface::name()],

            _ => panic!("Unsupported wsi"),
        }
    }

    fn create_surface(&self, entry: &ash::Entry, instance: &ash::Instance) -> ash::prelude::VkResult<vk::SurfaceKHR> {
        match (self.raw_window_handle().unwrap(), self.raw_display_handle().unwrap()) {
            #[cfg(target_os = "windows")]
            (RawWindowHandle::Windows(handle), _) => {
                let surface_desc = vk::Win32SurfaceCreateInfoKHR::builder()
                    .hinstance(handle.hinstance)
                    .hwnd(handle.hwnd);
                let surface_fn = khr::Win32Surface::new(entry, instance);
                Ok(surface_fn.create_win32_surface(&surface_desc, allocation_callbacks))
            }

            #[cfg(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ))]
            (RawWindowHandle::Wayland(handle), RawDisplayHandle::Wayland(display)) => {
                let surface_desc = vk::WaylandSurfaceCreateInfoKHR::builder()
                    .display(display.display.cast().as_ptr())
                    .surface(handle.surface.cast().as_ptr());
                let surface_fn = khr::WaylandSurface::new(entry, instance);
                unsafe { surface_fn.create_wayland_surface(&surface_desc, None) }
            }

            #[cfg(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ))]
            (RawWindowHandle::Xlib(handle), RawDisplayHandle::Xlib(display)) => {
                let surface_desc = vk::XlibSurfaceCreateInfoKHR::builder()
                    .dpy(display.display.unwrap().cast().as_ptr())
                    .window(handle.window);
                let surface_fn = khr::XlibSurface::new(entry, instance);
                unsafe { surface_fn.create_xlib_surface(&surface_desc, None) }
            }

            #[cfg(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            ))]
            (RawWindowHandle::Xcb(handle), RawDisplayHandle::Xcb(display)) => {
                let surface_desc = vk::XcbSurfaceCreateInfoKHR::builder()
                    .connection(display.connection.unwrap().cast().as_ptr())
                    .window(handle.window.into());
                let surface_fn = khr::XcbSurface::new(entry, instance);
                unsafe { surface_fn.create_xcb_surface(&surface_desc, None) }
            }

            _ => panic!("unsupported platform"),
        }
    }
}
impl<T: HasRawWindowHandle + HasRawDisplayHandle + WindowSize> WindowInterface for T {}
