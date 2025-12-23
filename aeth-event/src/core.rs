use futures::future::LocalBoxFuture;

/// AsyncHandlerTrait that is dyn-compatible.
///
/// The original AsyncFn(E) is not dyn-compatible,
/// but the event framework requires to dynamically
/// dispatch events to the handlers, so we must pay
/// the price of boxing the futures.
pub trait AsyncHandlerTrait<E> {
    fn call_mut_boxed(&mut self, e: E) -> LocalBoxFuture<'_, ()>;
}

impl<E, F> AsyncHandlerTrait<E> for F
where
    F: AsyncFnMut(E),
    E: Clone + 'static,
{
    fn call_mut_boxed(&mut self, e: E) -> LocalBoxFuture<'_, ()> {
        // This is the trick, we create an outer
        // future to unify the CallRefFuture types.
        Box::pin(async move { (*self)(e).await })
    }
}

/// Handler for dynamically dispatching events.
pub enum Handler<E> {
    Sync(Box<dyn FnMut(E)>),
    Async(Box<dyn AsyncHandlerTrait<E>>),
}

impl<E> Handler<E>
where
    E: Clone + 'static,
{
    pub fn new_sync<F>(f: F) -> Handler<E>
    where
        F: FnMut(E) + 'static,
    {
        Handler::Sync(Box::new(f))
    }

    pub fn new_sync_option<F>(mut f: F) -> Handler<E>
    where
        F: FnMut(E) -> Option<()> + 'static,
    {
        Handler::new_sync(move |e| {
            f(e);
        })
    }

    pub fn new_async<F>(f: F) -> Handler<E>
    where
        F: AsyncFnMut(E) + 'static,
    {
        Handler::Async(Box::new(f))
    }

    pub fn new_async_option<F>(mut f: F) -> Handler<E>
    where
        F: AsyncFnMut(E) -> Option<()> + 'static,
    {
        Handler::new_async(async move |e| {
            f(e).await;
        })
    }
}

#[cfg(test)]
mod test {
    use crate::core::Handler;
    #[test]
    fn test_define() {
        let _ = Handler::new_sync(move |v: usize| {
            println!("{}", v);
        });

        let _ = Handler::new_async(async move |v: usize| {
            println!("{}", v);
        });
    }
}
