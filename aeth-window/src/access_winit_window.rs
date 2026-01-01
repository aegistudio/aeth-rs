/// Forward method from [`winit::window::Window`] to an
/// extension of [`AccessWinitWindow`] trait.
pub use aeth_window_macros::forward_winit_window_method;
use winit::window::Window as WinitWindow;

/// Trait to access to its internally owned
/// [`winit::window::Window`] object.
///
/// This trait is made so that the user of this package
/// can write
/// `trait AccessWinitWindowExt : AccessWinitWindow`
/// plus
/// `impl<T> AccessWinitWindowExt for T where T: AccessWinitWindow {}`
/// to extend the behaviour of our `Window` object.
///
/// Please notice not only the holder of Window but
/// also the aeth-window subsystem will need to
/// access the underlying [`winit::window::Window`]
/// object. The holder is always an async task, and
/// allowing it to hold a [`std::cell::Ref`] to the
/// [`winit::window::Window`] can cause the window
/// subsystem to fail when they also borrow an
/// instance of it. So we enforce short-lived
/// reference access here by forcing the operation
/// on [`winit::window::Window`] to be completed
/// within a function.
pub trait AccessWinitWindow {
    /// Manipulate the underlying [`winit::window::Window`]
    /// in an immutable manner.
    fn map_winit_window<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&Box<dyn WinitWindow>) -> T;

    /// Manipulate the underlying [`winit::window::Window`]
    /// in a mutable manner.
    fn map_winit_window_mut<F, T>(&mut self, f: F) -> T
    where
        F: FnOnce(&mut Box<dyn WinitWindow>) -> T;
}

/// Trait extension to forawrd
/// [`winit::window::Window`] methods.
///
/// Please notice that we forward method based on the
/// requirement of other modules from aeth-rs, and not
/// necessarily all methods are forwarded. In that case,
/// you may add your required method to this trait
/// extension through pull request, or implement your
/// own extension, just like what we are doing here.
pub trait AccessWinitWindowExt: AccessWinitWindow {
    #[forward_winit_window_method]
    fn id(&self) -> winit::window::WindowId {
        todo!()
    }

    #[forward_winit_window_method]
    fn scale_factor(&self) -> f64 {
        todo!()
    }

    #[forward_winit_window_method]
    fn pre_present_notify(&self) {
        todo!()
    }

    #[forward_winit_window_method]
    fn surface_position(&self) -> winit::dpi::PhysicalPosition<i32> {
        todo!()
    }

    #[forward_winit_window_method]
    fn outer_position(
        &self,
    ) -> Result<winit::dpi::PhysicalPosition<i32>, winit::error::RequestError> {
        todo!()
    }

    #[forward_winit_window_method]
    fn set_outer_position(&self, position: winit::dpi::Position) {
        todo!()
    }

    #[forward_winit_window_method]
    fn surface_size(&self) -> winit::dpi::PhysicalSize<u32> {
        todo!()
    }

    #[forward_winit_window_method]
    fn request_surface_size(
        &self,
        size: winit::dpi::Size,
    ) -> Option<winit::dpi::PhysicalSize<u32>> {
        todo!()
    }

    #[forward_winit_window_method]
    fn outer_size(&self) -> winit::dpi::PhysicalSize<u32> {
        todo!()
    }

    #[forward_winit_window_method]
    fn safe_area(&self) -> winit::dpi::PhysicalInsets<u32> {
        todo!()
    }

    #[forward_winit_window_method]
    fn set_min_surface_size(&self, min_size: Option<winit::dpi::Size>) {
        todo!()
    }

    #[forward_winit_window_method]
    fn set_max_surface_size(&self, min_size: Option<winit::dpi::Size>) {
        todo!()
    }

    #[forward_winit_window_method]
    fn surface_resize_increments(&self) -> Option<winit::dpi::PhysicalSize<u32>> {
        todo!()
    }

    #[forward_winit_window_method]
    fn set_surface_resize_increments(&self, increments: Option<winit::dpi::Size>) {
        todo!()
    }

    #[forward_winit_window_method]
    fn title(&self) -> String {
        todo!()
    }

    #[forward_winit_window_method]
    fn set_title(&self, title: &str) {
        todo!()
    }

    #[forward_winit_window_method]
    fn set_transparent(&self, transparent: bool) {
        todo!()
    }
}

impl<T> AccessWinitWindowExt for T where T: AccessWinitWindow {}
