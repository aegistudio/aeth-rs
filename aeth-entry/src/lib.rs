use aeth_task::ready_poll::local_ready_poll;
use aeth_task::{Handle, framework, spawn_foreground};

/// Result type that is forwarded to make it easy
/// for code generation.
pub type Result<T> = anyhow::Result<T>;

/// Intended entrypoint of aeth-rs.
pub fn entrypoint<F>(future: F) -> Result<()>
where
    F: Future<Output = Result<()>> + 'static,
{
    let taskfx_cfg = framework::Config::default();
    let taskfx = framework::initialize(taskfx_cfg)?;

    let (main, poll) = local_ready_poll(future);
    spawn_foreground(main).detach();

    aeth_window::subsystem::run(taskfx, poll)?;
    Ok(())
}

pub use aeth_entry_macros::main;
