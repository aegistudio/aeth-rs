use aeth::event::prelude::*;
use aeth::window::AccessWinitActiveEventLoopExt;
use aeth::window::AccessWinitWindowExt;
use anyhow::Result;
use winit::event::WindowEvent;
use winit::event_loop::ControlFlow;
use winit::window::WindowAttributes;

#[aeth::main]
async fn main() -> Result<()> {
    let manager = aeth::window::manager().await;
    manager.set_control_flow(ControlFlow::Wait).await;
    let mut window_attrs = WindowAttributes::default();
    window_attrs.title = "Blank Window".to_string();
    let mut window = manager.create_window(window_attrs).await?;
    println!("Evaluated window ID: {:?}", window.id());
    println!("Evaluated window outer size: {:?}", window.outer_size());
    println!("Evaluated window surface size: {:?}", window.surface_size());
    // NOTE: set REFRESHING to non-empty to enable refreshing.
    let refreshing = std::env::var_os("REFRESHING")
        .unwrap_or_default()
        .as_encoded_bytes()
        .trim_ascii()
        .len()
        > 0;
    window.set_refreshing(refreshing);

    let mut window_event_ch = window.window_event_sub().chan().await;
    loop {
        let event = window_event_ch.recv().await;
        println!("Receiving window event: {event:?}");
        match event {
            WindowEvent::CloseRequested => return Ok(()),
            _ => {}
        }
    }
}
