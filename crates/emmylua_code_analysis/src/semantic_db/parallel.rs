//! Order-preserving parallel map for the full-rebuild phases.
//!
//! The per-file builders of `rebuild_all_caches` only read the database, so each
//! phase can run its files on several threads and install the results afterwards,
//! in file order, on the calling thread. Output never depends on thread timing.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

// The parallel phases share `&SemanticDatabase` between threads and move their
// per-file results back; a non-thread-safe field would break this at compile time.
const _: fn() = || {
    fn sync<T: Sync>() {}
    fn send<T: Send>() {}
    sync::<super::SemanticDatabase>();
    send::<super::facts::FileFacts>();
    send::<super::flow::FlowTree>();
    send::<super::exports::FileExportContribution>();
    send::<super::query::FileReferences>();
};

/// Upper bound on index worker threads: beyond this the phases are bound by
/// allocator and cache contention, not by CPU.
const MAX_INDEX_THREADS: usize = 8;

/// Worker stack size. The builders recurse over syntax trees, so workers get more
/// than the 2 MiB default of the runtime threads that call `rebuild_all_caches`.
const WORKER_STACK_BYTES: usize = 8 * 1024 * 1024;

/// Number of threads (including the caller) a parallel phase may use.
///
/// `EMMYLUA_INDEX_THREADS=1` forces the sequential path, for debugging and for
/// bounding CPU use. Read once.
fn index_threads() -> usize {
    static THREADS: OnceLock<usize> = OnceLock::new();
    *THREADS.get_or_init(|| {
        if let Some(requested) = std::env::var("EMMYLUA_INDEX_THREADS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            && requested >= 1
        {
            return requested;
        }
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(MAX_INDEX_THREADS)
    })
}

/// `items.iter().map(f).collect()` on up to [`index_threads`] threads.
///
/// Items are handed out one at a time (files differ a lot in size), and the
/// results come back in `items` order. The calling thread works too, so a failure
/// to spawn a worker only costs speed.
pub(crate) fn par_map<T, R, F>(items: &[T], f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    par_map_with(index_threads(), items, f)
}

fn par_map_with<T, R, F>(max_threads: usize, items: &[T], f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    let threads = max_threads.min(items.len());
    if threads <= 1 {
        return items.iter().map(f).collect();
    }

    let next = AtomicUsize::new(0);
    let work = || {
        let mut done: Vec<(usize, R)> = Vec::new();
        loop {
            let index = next.fetch_add(1, Ordering::Relaxed);
            let Some(item) = items.get(index) else {
                break;
            };
            done.push((index, f(item)));
        }
        done
    };

    let mut results: Vec<(usize, R)> = Vec::with_capacity(items.len());
    std::thread::scope(|scope| {
        let workers: Vec<_> = (1..threads)
            .filter_map(|worker| {
                std::thread::Builder::new()
                    .name(format!("emmylua-index-{worker}"))
                    .stack_size(WORKER_STACK_BYTES)
                    .spawn_scoped(scope, &work)
                    .inspect_err(|err| log::warn!("cannot start index worker {worker}: {err}"))
                    .ok()
            })
            .collect();
        results.extend(work());
        for worker in workers {
            match worker.join() {
                Ok(done) => results.extend(done),
                Err(payload) => std::panic::resume_unwind(payload),
            }
        }
    });
    results.sort_unstable_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, value)| value).collect()
}

#[cfg(test)]
mod tests {
    use super::{par_map, par_map_with};

    #[test]
    fn par_map_keeps_input_order() {
        let items: Vec<u32> = (0..1000).collect();
        let expected: Vec<u32> = items.iter().map(|n| n * 2).collect();
        for threads in [1, 2, 4, 16] {
            // Uneven per-item cost so workers finish out of order.
            let doubled = par_map_with(threads, &items, |n| {
                if n % 7 == 0 {
                    std::thread::yield_now();
                }
                n * 2
            });
            assert_eq!(doubled, expected, "threads = {threads}");
        }
    }

    #[test]
    fn par_map_handles_empty_and_single() {
        assert!(par_map::<u32, u32, _>(&[], |n| *n).is_empty());
        assert_eq!(par_map(&[7u32], |n| n + 1), vec![8]);
    }
}
