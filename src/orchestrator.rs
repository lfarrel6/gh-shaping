/// A generic work-distribution interface.
///
/// `Item` is the unit of work sent to each worker; `Output` is what the worker
/// returns.  Placing the type parameters on the *trait* (not on `run`) keeps
/// the trait object-safe, so callers can hold a `Box<dyn Orchestrate<I, O>>`
/// and switch strategies at runtime for a fixed item/output pair.
///
/// For runtime dispatch across *different* item/output types in the same
/// binary, use [`Strategy`], which wraps the two built-in implementations and
/// exposes a generic `run` method without trait objects.
pub trait Orchestrate<Item: Send, Output: Send>: Send + Sync {
    fn run(
        &self,
        items: Vec<Item>,
        worker: &(dyn Fn(Item) -> Output + Send + Sync),
    ) -> Vec<Output>;
}

// ── built-in implementations ──────────────────────────────────────────────────

/// Processes items one at a time on the calling thread.
pub struct Sequential;

/// Spawns one scoped thread per item.
///
/// [`std::thread::scope`] guarantees all threads finish before `run` returns,
/// so workers can borrow from the caller's stack without `Arc` or `'static`.
pub struct Parallel;

impl<Item: Send, Output: Send> Orchestrate<Item, Output> for Sequential {
    fn run(
        &self,
        items: Vec<Item>,
        worker: &(dyn Fn(Item) -> Output + Send + Sync),
    ) -> Vec<Output> {
        items.into_iter().map(|item| worker(item)).collect()
    }
}

impl<Item: Send, Output: Send> Orchestrate<Item, Output> for Parallel {
    fn run(
        &self,
        items: Vec<Item>,
        worker: &(dyn Fn(Item) -> Output + Send + Sync),
    ) -> Vec<Output> {
        std::thread::scope(|scope| {
            let handles: Vec<_> = items
                .into_iter()
                .map(|item| scope.spawn(|| worker(item)))
                .collect();

            handles
                .into_iter()
                .map(|h| h.join().expect("worker thread panicked"))
                .collect()
        })
    }
}

// ── runtime strategy ──────────────────────────────────────────────────────────

/// Selects between the two built-in implementations at runtime.
///
/// Unlike a trait object, `Strategy` can be reused across calls with different
/// `Item`/`Output` type parameters — it delegates to the same underlying
/// `Sequential` or `Parallel` struct each time.
pub enum Strategy {
    Sequential,
    Parallel,
}

impl Strategy {
    pub fn run<Item: Send, Output: Send>(
        &self,
        items: Vec<Item>,
        worker: &(dyn Fn(Item) -> Output + Send + Sync),
    ) -> Vec<Output> {
        match self {
            Strategy::Sequential => Sequential.run(items, worker),
            Strategy::Parallel => Parallel.run(items, worker),
        }
    }
}
