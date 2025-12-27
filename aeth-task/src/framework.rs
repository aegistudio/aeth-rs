//! Manipulation of the task framework itself.
//!
//! Normally the task module user does not need to care
//! about manipulating the task framework, so we move it
//! to a submodule to avoid overwhelming them.

use crate::foreground;
use crate::spawner::{BackgroundSpawner, ForegroundSpawner, SPAWNER, Spawner};
use anyhow::Result;
use core_affinity::{CoreId, get_core_ids, set_for_current};
use futures::lock::Mutex;
use futures::{channel::mpsc::unbounded, executor::LocalPool};
use rand::seq::IndexedRandom;
use std::cell::RefCell;
use std::cmp::{max, min};
use thread_priority::{ThreadPriority, set_current_thread_priority};

/// Configuration for the task framework.
#[derive(Default)]
pub struct Config {
    pub min_background_threads: Option<usize>,
    pub max_background_threads: Option<usize>,
    pub num_background_threads: Option<usize>,
}

/// Currently initialized threading mode.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThreadingMode {
    /// Disjoint mode is the ideal mode that
    /// multiple CPUs are available, and thus
    /// foreground thread and background threads
    /// may run on disjoint set of CPUs.
    ///
    /// This is the mode set whenever there're
    /// 2 or more CPUs permitted.
    Disjoint,

    /// Contended mode is the downgrade mode that
    /// only single CPU is available, and
    /// foreground thread and background threads
    /// have to run on that CPU.
    ///
    /// This mode is set when there's single CPU
    /// or we are unable to get permitted CPU set.
    Contended,
}

/// Task framework initialization status.
pub struct Status {
    threading_mode: ThreadingMode,
}

impl Status {
    pub fn threading_mode(&self) -> ThreadingMode {
        self.threading_mode
    }
}

#[derive(Clone)]
enum ThreadInitializer {
    Disjoint { cores: Vec<CoreId> },
    Contended,
}

impl ThreadInitializer {
    fn configure_on_foreground(&self) {
        match self {
            Self::Disjoint { cores } => {
                set_for_current(cores[0]);
                let _ = set_current_thread_priority(ThreadPriority::Max);
                std::thread::yield_now();
            }
            Self::Contended => {
                let _ = set_current_thread_priority(ThreadPriority::Max);
            }
        }
    }

    fn configure_on_background(&self) {
        match self {
            Self::Disjoint { cores } => {
                assert!(cores.len() > 1);
                let mut rng = rand::rng();
                let core = cores[1..].choose(&mut rng);
                set_for_current(core.unwrap().clone());
            }
            Self::Contended => {
                let _ = set_current_thread_priority(ThreadPriority::Min);
            }
        }
    }

    fn recommend_num_backgrounds(&self) -> usize {
        match self {
            Self::Disjoint { cores } => cores.len() - 1,
            Self::Contended => 1,
        }
    }

    fn threading_mode(&self) -> ThreadingMode {
        match self {
            Self::Disjoint { cores: _ } => ThreadingMode::Disjoint,
            Self::Contended => ThreadingMode::Contended,
        }
    }
}

/// Initialized framework handle.
/// 
/// This handle is used to control the lifecycle
/// of the whole system. By dropping it, you
/// dispose the entire framework.
/// 
/// Disposing the framework is required on some
/// platform such as windows.
pub struct Framework {
    status: Status,
}

impl Framework {
    pub fn status(&self) -> &Status {
        &self.status
    }
}

impl Drop for Framework {
    fn drop(&mut self) {
        foreground::assert();
        drop(SPAWNER.replace(Spawner::Uninit))
    }
}

/// Initialize the task framework.
///
/// Please notice that the tasking system will only
/// be initialized once, and the later invocation
/// will always return the result of the first
/// invocation, ignoring the config.
pub fn initialize(cfg: Config) -> Result<Framework> {
    SPAWNER.with_borrow(|v| {
        match v {
            &Spawner::Uninit => Ok(()),
            _ => Err(anyhow::anyhow!("Initialized framework in use."))
        }
    })?;

    let initializer = {
        let cores = get_core_ids().unwrap_or(Vec::new());
        if cores.len() >= 2 {
            ThreadInitializer::Disjoint { cores }
        } else {
            ThreadInitializer::Contended
        }
    };

    let mut num_workers = initializer.recommend_num_backgrounds();
    num_workers = max(1, num_workers);
    if let Some(min_background_threads) = cfg.min_background_threads {
        num_workers = max(min_background_threads, num_workers);
    }
    if let Some(max_background_threads) = cfg.max_background_threads {
        num_workers = min(max_background_threads, num_workers);
    }
    if let Some(num_background_threads) = cfg.num_background_threads {
        num_workers = num_background_threads;
    }

    let (loopback_send, loopback_recv) = unbounded();

    let thread_send = loopback_send.clone();
    let thread_initializer = initializer.clone();
    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(num_workers)
        .on_thread_start(move || {
            thread_initializer.configure_on_background();
            SPAWNER.with_borrow_mut(|v| {
                *v = Spawner::Background(BackgroundSpawner {
                    loopback_send: thread_send.clone(),
                });
            })
        })
        .enable_all()
        .build()?;

    initializer.configure_on_foreground();
    SPAWNER.with_borrow_mut(|v| {
        let local_pool = LocalPool::new();
        let local_spawner = local_pool.spawner();
        *v = Spawner::Foreground(ForegroundSpawner {
            local_pool: RefCell::new(local_pool),
            local_spawner: local_spawner,
            tokio_runtime: tokio_runtime,
            loopback_recv: Mutex::new(loopback_recv),
            _loopback_send: loopback_send,
        });
    });

    Ok(Framework {
        status: Status {
            threading_mode: initializer.threading_mode(),
        },
    })
}
