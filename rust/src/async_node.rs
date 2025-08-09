use std::rc::Rc;

use async_compat::CompatExt;
use async_executor::LocalExecutor;
use flume::unbounded;
use godot::{
    builtin::{Callable, Variant},
    classes::{Engine, SceneTree},
    obj::{Gd, WithBaseField},
    prelude::GodotClass,
};
use tracing::{Instrument, Span};

/// Implement this trait for your Godot class to make an async executor whose lifetime is bound to your class.
/// That means - the executor (and all its tasks) will automatically be stopped if the object gets freed.
/// This has the added advantage of making it safe to use `self` in the future, without having to check every time if `self` is destroyed.
/// Two other advantages of doing it this way:
/// 1. The tasks will be paused if you temporarily remove the node from the scene tree.
///    Very useful for FSMs or object pools!
/// 2. The tasks can be woken up by other threads, without requiring the `experimental-threads` feature.
///    Very useful for receiving messages on channels from other threads!
///
/// This behaves similarly to Tween.bind_node(), except we bind an async executor, instead of a tween.
/// Note - do not forget to call `self.tick_deferred()` in `process` (or `physics_process` if you want)
pub trait AsyncNode {
    /// Note - this does not overwrite the previous executor, so we can have multiple tasks running concurrently
    fn spawn_local_task<
        R: Future<Output = O> + 'static,
        U: FnOnce(Gd<Self>) -> R + 'static,
        O: 'static,
    >(
        &mut self,
        use_tokio_compat_bridge: bool,
        span: Span,
        future: U,
    ) where
        Self: GodotClass + WithBaseField,
    {
        let this: Gd<Self> = self.to_gd();

        let executor = match self.get_executor() {
            Some(exe) => exe,
            None => {
                let temp = Rc::new(LocalExecutor::new());
                self.set_executor(Some(Rc::clone(&temp)));
                &Rc::clone(&temp)
            }
        };

        // TODO: make the span optional?

        let span = span.or_current();
        if use_tokio_compat_bridge {
            executor
                .spawn(async move { future(this).instrument(span).compat().await })
                .detach();
        } else {
            executor
                .spawn(async move { future(this).instrument(span).await })
                .detach();
        }
    }

    /// Store the executor in the Rust side of the node, to ensure it's cleaned up when the node is destroyed
    fn set_executor(&mut self, executor: Option<Rc<LocalExecutor<'static>>>);
    fn get_executor(&self) -> &Option<Rc<LocalExecutor<'static>>>;

    /// Cancels all the currently running async tasks instantly.
    /// This is not recommended - ideally you want to pass a CancellationToken into every async task, so it can be cancelled gracefully.
    /// Otherwise the program may be left in a slightly invalid state.
    fn stop_all_tasks(&mut self) {
        self.set_executor(None);
    }

    /// Ticks the executor via call_deferred. Should still happen on the same frame if you call this in _process.
    /// Note - this uses a while loop, so don't `yield_now` in your async task, else you get an infinite loop.
    fn tick_deferred(&mut self) {
        if let Some(exec) = &self.get_executor() {
            let exec = Rc::clone(exec);
            Callable::from_local_fn("ticker", move |_| {
                while exec.try_tick() {
                    // Do nothing.
                    // See https://www.reddit.com/r/rust/comments/1k0f174/comment/mnfyr2l/
                }

                Ok(Variant::nil())
            })
            .call_deferred(&[]);

            // TODO: `self.call_deferred()` would be probably better, to make sure it doesn't get called if `self` is freed.
            // Update: in gdext 0.3.3 we now have apply_deferred which is type safe, no more strings!
            // But we may run into the `exec.try_tick() triggers borrow error` bug again...
            // Except this time, we're using `experimental-threads`, which means your Rust structs are protected by a Mutex instead of a RefCell.
            // So, this time, it may actually deadlock instead of triggering a borrow panic...  Bad solution!
            // Instead of `this = self.to_gd()` try storing self.base() instead? Then cast it to your Rust class in the async fn?
        }
    }
}

/// Set ignore_time_scale to true to make things happen in real-time, ignoring slow motion.
/// Note - you HAVE to make the method itself async, instead of returning `impl Future`.
/// Otherwise you get `ERROR: Parameter "obj" is null.`
pub async fn wait_for_duration(time: f64, ignore_time_scale: bool) {
    Engine::singleton()
        .get_main_loop()
        .unwrap()
        .cast::<SceneTree>()
        .create_timer_ex(time)
        .ignore_time_scale(ignore_time_scale)
        .done()
        .unwrap()
        .signals()
        .timeout()
        .to_future()
        .await
}

pub async fn wait_for_next_frame() {
    Engine::singleton()
        .get_main_loop()
        .unwrap()
        .cast::<SceneTree>()
        .signals()
        .process_frame()
        .to_future()
        .await
}

pub async fn wait_for_next_physics_frame() {
    Engine::singleton()
        .get_main_loop()
        .unwrap()
        .cast::<SceneTree>()
        .signals()
        .physics_frame()
        .to_future()
        .await
}

/// Spawns a Rayon task that runs F and awaits it.
/// Returns flume::RecvError if F panicked.
pub fn spawn_rayon_with_result<R, F>(func: F) -> impl Future<Output = Result<R, flume::RecvError>>
where
    R: Send + 'static,
    F: FnOnce() -> R + Send + 'static,
{
    let (tx, rx) = unbounded();

    rayon::spawn(move || {
        let result = func();
        let sent = tx.send(result);

        if let Err(err) = sent {
            tracing::warn!(
                ?err,
                "Failed to send result of calculation in spawn_with_result"
            );
        }
    });

    // Little hack to move rx into an async block, so we can return a reference to it
    async move { rx.recv_async().await }
}
