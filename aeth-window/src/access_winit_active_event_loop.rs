/// Forward method from [`winit::event_loop::ActiveEventLoop`] to an
/// extension of [`AccessWinitActiveEventLoop`] trait.
pub use aeth_window_macros::forward_winit_active_event_loop_method;
use winit::event_loop::ActiveEventLoop;

/// Trait to access [`winit::event_loop::ActiveEventLoop`].
///
/// In fact, the active event loop is not available all
/// the time, so accessing the active event loop will
/// require the window subsystem to put them in correct
/// timing. Thus the accessor must be asynchronous.
#[allow(async_fn_in_trait)]
pub trait AccessWinitActiveEventLoop {
    /// Run the function in the place where the active event
    /// loop is available.
    async fn run_with_active_event_loop<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&dyn ActiveEventLoop) -> T + 'static,
        T: 'static;
}

/// Trait extension to forawrd
/// [`winit::event_loop::ActiveEventLoop`] methods.
///
/// Please notice
/// [`winit::event_loop::ActiveEventLoop::create_window`]
/// will never be forwarded. You must always not
/// do that, instead you must call
/// [`crate::Manager::create_window`]
/// to get things right.
///
/// The
/// [`winit::event_loop::ActiveEventLoop::exit`]
/// is not forwarded either. We treat the exiting
/// of the main coroutine as exiting the event loop.
#[allow(async_fn_in_trait)]
pub trait AccessWinitActiveEventLoopExt: AccessWinitActiveEventLoop {
    #[forward_winit_active_event_loop_method]
    async fn create_proxy(&self) -> winit::event_loop::EventLoopProxy {
        todo!()
    }

    #[forward_winit_active_event_loop_method]
    async fn create_custom_cursor(
        &self,
        custom_cursor: winit::cursor::CustomCursorSource,
    ) -> Result<winit::cursor::CustomCursor, winit::error::RequestError> {
        todo!()
    }

    #[forward_winit_active_event_loop_method]
    async fn available_monitors(&self) -> Box<dyn Iterator<Item = winit::monitor::MonitorHandle>> {
        todo!()
    }

    #[forward_winit_active_event_loop_method]
    async fn primary_monitor(&self) -> Option<winit::monitor::MonitorHandle> {
        todo!()
    }

    #[forward_winit_active_event_loop_method]
    async fn listen_device_events(&self, allowed: winit::event_loop::DeviceEvents) {
        todo!()
    }

    #[forward_winit_active_event_loop_method]
    async fn system_theme(&self) -> Option<winit::window::Theme> {
        todo!()
    }

    #[forward_winit_active_event_loop_method]
    async fn set_control_flow(&self, flow: winit::event_loop::ControlFlow) {
        todo!()
    }

    #[forward_winit_active_event_loop_method]
    async fn control_flow(&self) -> winit::event_loop::ControlFlow {
        todo!()
    }

    #[forward_winit_active_event_loop_method]
    async fn owned_display_handle(&self) -> winit::event_loop::OwnedDisplayHandle {
        todo!()
    }
}

impl<T> AccessWinitActiveEventLoopExt for T where T: AccessWinitActiveEventLoop {}
