//! Pipelines compiling while an engine is put together.

/// A pipeline compiling on the thread pool.
///
/// Compiling shaders is nearly all of what building an engine costs and no
/// pipeline waits on another, so an engine starts all of its own first and
/// collects them as it is put together.
pub(crate) struct Compiling<T> {
    done: std::sync::mpsc::Receiver<T>,
    compile: std::sync::Arc<dyn Fn() -> T + Send + Sync>,
}

impl<T: Send + 'static> Compiling<T> {
    pub(crate) fn start(compile: impl Fn() -> T + Send + Sync + 'static) -> Self {
        let compile: std::sync::Arc<dyn Fn() -> T + Send + Sync> = std::sync::Arc::new(compile);
        let (send, done) = std::sync::mpsc::channel();
        let job = std::sync::Arc::clone(&compile);
        // The browser build has no pool to compile on, so `wait` compiles it.
        #[cfg(not(target_arch = "wasm32"))]
        rayon::spawn(move || {
            let _ = send.send(job());
        });
        #[cfg(target_arch = "wasm32")]
        let _ = (send, job);
        Self { done, compile }
    }

    /// The pipeline, compiled here if the pool never answered.
    // ponytail: this blocks, so an engine built from inside the pool on a
    // one-core machine would wait on itself. Nothing builds one there.
    pub(crate) fn wait(self) -> T {
        self.done.recv().unwrap_or_else(|_| (self.compile)())
    }
}
