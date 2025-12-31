use aeth_event::{Pub, Sub, new_pubsub};
use indexed_bitmap::IndexedBitmap;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use winit::event::WindowEvent;
use winit::window::{Window as WinitWindow, WindowId};

pub(crate) struct WindowInner {
    window: Box<dyn WinitWindow>,
    window_event_pub: Pub<WindowEvent>,
    win_id: WindowId,
    slot_id: Rc<RefCell<usize>>,
}

impl WindowInner {
    pub(crate) fn window_event_pub(&self) -> Pub<WindowEvent> {
        self.window_event_pub.clone()
    }
}

pub(crate) struct Windows {
    windows: Vec<WindowInner>,
    lookup: HashMap<WindowId, Rc<RefCell<usize>>>,
    refreshing: IndexedBitmap,
}

impl Windows {
    pub(crate) fn new() -> Self {
        Self {
            windows: Vec::new(),
            lookup: HashMap::new(),
            refreshing: IndexedBitmap::new(),
        }
    }

    fn unlink(&mut self, id: usize) {
        let evicted_id = self.windows[id].win_id;
        self.lookup.remove(&evicted_id);
        self.refreshing.bitset(id, false);
    }

    pub(crate) fn evict(&mut self, id: usize) {
        // Since this is called from a window holder,
        // who allocates a window.
        assert!(self.windows.len() > 0);
        self.unlink(id);
        let last_id = self.windows.len() - 1;
        if id < last_id {
            let win_id = self.windows[last_id].win_id;
            let slot_id = self.windows[last_id].slot_id.clone();
            let last_refreshing = self.refreshing.bitget(last_id);
            self.unlink(last_id);
            self.windows.swap(id, last_id);
            *slot_id.borrow_mut() = id;
            self.refreshing.bitset(id, last_refreshing);
            self.lookup.insert(win_id, slot_id);
        }
        // The node to index has been swapped to the
        // last item, ready to remove.
        self.windows.pop();
        if self.windows.len() * 2 < self.windows.capacity() {
            self.windows.shrink_to_fit();
            self.refreshing.shrink_to(self.windows.capacity());
        }
    }

    fn set_refreshing(&mut self, id: usize, enable: bool) {
        self.refreshing.bitset(id, enable);
    }

    fn broadcast_refresh_option(&self) -> Option<()> {
        let mut id = self.refreshing.lowest_one()?;
        loop {
            self.windows[id].window.request_redraw();
            id = self.refreshing.next_one(id)?;
        }
    }

    pub(crate) fn broadcast_refresh(&self) {
        self.broadcast_refresh_option();
    }

    pub(crate) fn allocate(rc: &Rc<RefCell<Self>>, window: Box<dyn WinitWindow>) -> Window {
        let mut this = rc.borrow_mut();
        let win_id = window.id();
        let slot_id = this.windows.len();
        let slot_id = Rc::new(RefCell::new(slot_id));
        let (window_event_pub, window_event_sub) = new_pubsub();
        this.windows.push(WindowInner {
            window,
            window_event_pub,
            win_id,
            slot_id: slot_id.clone(),
        });
        this.lookup.insert(win_id, slot_id.clone());
        Window {
            windows: Rc::downgrade(rc),
            window_event_sub,
            slot_id: slot_id,
        }
    }

    pub(crate) fn find_inner_by_window_id(
        &mut self,
        window_id: WindowId,
    ) -> Option<&mut WindowInner> {
        let slot_id = *self.lookup.get(&window_id)?.borrow();
        Some(&mut self.windows[slot_id])
    }
}

/// Managed window handle.
///
/// Things like redrawing a window,
/// accepting a user interaction, are
/// passed by window events. A graphical
/// program runs an event loop to poll
/// these events and handles them.
/// The window subsystem mediates the
/// interaction between the event loop
/// and async task process. When a
/// window event happens, it will need
/// to locate the window to deliver
/// the window event. This is impossible
/// without getting the windows managed.
///
/// Therefore, we need such an object.
/// It's built on the top of an
/// `winit::window::Window`, registered
/// to the window subsystem and expose
/// a subscriber `window_event_sub` for
/// receiving window events dedicated
/// to this window.
pub struct Window {
    windows: Weak<RefCell<Windows>>,
    window_event_sub: Sub<WindowEvent>,
    slot_id: Rc<RefCell<usize>>,
}

impl Drop for Window {
    fn drop(&mut self) {
        self.drop_option();
    }
}

impl Window {
    fn id(&self) -> usize {
        *self.slot_id.borrow()
    }

    fn drop_option(&mut self) -> Option<()> {
        let rc = self.windows.upgrade()?;
        rc.borrow_mut().evict(self.id());
        Some(())
    }

    fn must_upgrade_windows(&self) -> Rc<RefCell<Windows>> {
        self.windows
            .upgrade()
            .expect("Windows dropped, maybe not inside valid window subsystem context")
    }

    /// Manipulate the underlying `winit::window::Window`
    /// in an immutable manner.
    pub fn map_winit_window<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&Box<dyn WinitWindow>) -> T,
    {
        let rc = self.must_upgrade_windows();
        let windows = rc.borrow();
        let window = &windows.windows[self.id()].window;
        f(window)
    }

    /// Manipulate the underlying `winit::window::Window`
    /// in a mutable manner.
    pub fn map_winit_window_mut<F, T>(&mut self, f: F) -> T
    where
        F: FnOnce(&mut Box<dyn WinitWindow>) -> T,
    {
        // XXX: on the exclusive access to the &mut Windows,
        // since this is executed no the foreground thread,
        // the function is sync and returns immediately,
        // and the mutable reference will be dropped shortly,
        // it's okay to do so. Also we require the exclusive
        // ownership of Window to perform exclusive operations
        // on the winit window.
        let rc = self.must_upgrade_windows();
        let mut windows = rc.borrow_mut();
        let window = &mut windows.windows[self.id()].window;
        f(window)
    }

    /// Set the window as being refreshing or not.
    ///
    /// By setting this window as being refreshing,
    /// every time the event loop is about to sleep,
    /// a redraw event is going to be queued for
    /// this window. This is required for video
    /// applications and games.
    pub fn set_refreshing(&mut self, enable: bool) {
        let rc = self.must_upgrade_windows();
        let mut windows = rc.borrow_mut();
        windows.set_refreshing(self.id(), enable);
    }

    // Check if the window is refreshing or not.
    pub fn is_refreshing(&self) -> bool {
        let rc = self.must_upgrade_windows();
        let windows = rc.borrow_mut();
        windows.refreshing.bitget(self.id())
    }

    /// Obtain the window event subscriber dedicated
    /// to this window.
    pub fn window_event_sub(&self) -> Sub<WindowEvent> {
        self.window_event_sub.clone()
    }
}
