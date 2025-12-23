use std::cell::RefCell;
use std::rc::Rc;

use futures::{
    executor::{LocalPool, LocalSpawner},
    task::LocalSpawnExt,
};

pub(crate) struct Execute {
    done: Rc<RefCell<bool>>,
}

impl Execute {
    pub(crate) fn assert_done(&self) {
        assert!(*self.done.borrow(), "task is not done");
    }
}

pub(crate) struct TestFixture {
    pool: LocalPool,
    spawner: LocalSpawner,
}

impl TestFixture {
    pub(crate) fn new() -> Self {
        let pool = LocalPool::new();
        let spawner = pool.spawner();
        Self {
            pool: pool,
            spawner: spawner,
        }
    }

    pub(crate) fn execute<F>(&mut self, future: F) -> Execute
    where
        F: Future<Output = ()> + 'static,
    {
        let done = Rc::new(RefCell::new(false));
        let done_moved = done.clone();
        self.spawner
            .spawn_local(async move {
                future.await;
                *done_moved.borrow_mut() = true;
            })
            .unwrap();
        self.pool.run_until_stalled();
        Execute { done: done }
    }
}
