use aeth_event::Chan;
use aeth_task::ready_poll::local_ready_poll;
use aeth_task::{Handle, framework, spawn_foreground};
use aeth_window::AccessWinitActiveEventLoopExt;
use aeth_window::AccessWinitWindowExt;
use anyhow::Result;
use winit::event::WindowEvent;
use winit::event_loop::ControlFlow;
use winit::window::WindowAttributes;

async fn async_main() -> Result<()> {
    let manager = aeth_window::manager().await;
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
    let window_event_sub = window.window_event_sub();
    let mut window_event_ch = Chan::new();
    let _sub = window_event_ch.connect(window_event_sub).await;

    loop {
        let event = window_event_ch.recv().await;
        println!("Receiving window event: {event:?}");
        match event {
            WindowEvent::CloseRequested => return Ok(()),
            _ => {}
        }
    }
}

fn main() -> Result<()> {
    let taskfx_cfg = framework::Config::default();
    let taskfx = framework::initialize(taskfx_cfg)?;

    let (main, poll) = local_ready_poll(async_main());
    spawn_foreground(main).detach();

    aeth_window::subsystem::run(taskfx, poll)?;
    Ok(())
}
