//! A small persistent thread pool. `run(n, f)` calls `f(0) .. f(n − 1)` across the
//! workers and the calling thread and returns once all are done. Which thread runs which
//! index varies; callers only hand out independent work (each index writes its own
//! output), so results never depend on the schedule or the thread count.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

type Task = dyn Fn(usize) + Sync;

thread_local! {
    /// set while this thread runs a task, so nested `run` calls stay on it
    static IN_TASK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[derive(Clone, Copy)]
struct Job(*const Task);
// SAFETY: the closure is Sync, and `run` keeps it alive until every call has returned
unsafe impl Send for Job {}

#[derive(Default)]
struct State {
    job: Option<Job>,
    next: usize,
    total: usize,
    finished: usize,
    panicked: bool,
    quit: bool,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    work: Condvar,
    done: Condvar,
}

pub struct Pool {
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
    gate: Mutex<()>,
}

/// threads std reports for this machine, at least 1
pub fn available() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

impl Pool {
    /// a pool of `threads` threads in all, the caller included (so `threads − 1` workers)
    pub fn new(threads: usize) -> Pool {
        let shared = Arc::new(Shared::default());
        let workers = (1..threads.max(1))
            .map(|_| {
                let s = shared.clone();
                std::thread::spawn(move || worker(&s))
            })
            .collect();
        Pool {
            shared,
            workers,
            gate: Mutex::new(()),
        }
    }

    pub fn threads(&self) -> usize {
        self.workers.len() + 1
    }

    /// calls `f(i)` for every i in 0..n, in parallel, and waits for all of them
    pub fn run(&self, n: usize, f: &(dyn Fn(usize) + Sync)) {
        if n == 0 {
            return;
        }
        if self.workers.is_empty() || n == 1 || IN_TASK.with(|t| t.get()) {
            (0..n).for_each(f);
            return;
        }
        // one job at a time; a task's own `run` calls go serial (see `IN_TASK`)
        let _gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: lifetime erased only until this function returns, and it returns
        // only after `finished == n`, when no worker holds the pointer any more
        let f: &'static Task = unsafe { std::mem::transmute::<&(dyn Fn(usize) + Sync), &'static Task>(f) };
        let job = Job(f as *const Task);
        {
            let mut st = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
            st.job = Some(job);
            st.next = 0;
            st.total = n;
            st.finished = 0;
            st.panicked = false;
        }
        self.shared.work.notify_all();
        take(&self.shared);
        let mut st = self.shared.state.lock().unwrap_or_else(|e| e.into_inner());
        while st.finished < st.total {
            st = self.shared.done.wait(st).unwrap_or_else(|e| e.into_inner());
        }
        st.job = None;
        let panicked = st.panicked;
        drop(st);
        if panicked {
            panic!("a pool task panicked");
        }
    }
}

/// claims and runs indices of the current job until none are left
fn take(s: &Shared) {
    loop {
        let (job, i) = {
            let mut st = s.state.lock().unwrap_or_else(|e| e.into_inner());
            match st.job {
                Some(job) if st.next < st.total => {
                    st.next += 1;
                    (job, st.next - 1)
                }
                _ => return,
            }
        };
        // SAFETY: see `run`
        let f = unsafe { &*job.0 };
        let was = IN_TASK.with(|t| t.replace(true));
        let ok = catch_unwind(AssertUnwindSafe(|| f(i))).is_ok();
        IN_TASK.with(|t| t.set(was));
        let mut st = s.state.lock().unwrap_or_else(|e| e.into_inner());
        st.finished += 1;
        st.panicked |= !ok;
        if st.finished == st.total {
            s.done.notify_all();
        }
    }
}

fn worker(s: &Shared) {
    loop {
        {
            let mut st = s.state.lock().unwrap_or_else(|e| e.into_inner());
            while !st.quit && !(st.job.is_some() && st.next < st.total) {
                st = s.work.wait(st).unwrap_or_else(|e| e.into_inner());
            }
            if st.quit {
                return;
            }
        }
        take(s);
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.shared.state.lock().unwrap_or_else(|e| e.into_inner()).quit = true;
        self.shared.work.notify_all();
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

/// a slice that pool tasks write disjoint parts of
pub struct Shards<'a, T> {
    ptr: *mut T,
    len: usize,
    _life: std::marker::PhantomData<&'a mut [T]>,
}

// SAFETY: tasks only touch disjoint ranges (the contract of `part`)
unsafe impl<T: Send> Sync for Shards<'_, T> {}
unsafe impl<T: Send> Send for Shards<'_, T> {}

impl<'a, T> Shards<'a, T> {
    pub fn new(s: &'a mut [T]) -> Shards<'a, T> {
        Shards {
            ptr: s.as_mut_ptr(),
            len: s.len(),
            _life: std::marker::PhantomData,
        }
    }

    /// # Safety
    /// no two live parts may overlap
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn part(&self, start: usize, len: usize) -> &mut [T] {
        assert!(start + len <= self.len);
        unsafe { std::slice::from_raw_parts_mut(self.ptr.add(start), len) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn runs_every_index_once() {
        let pool = Pool::new(4);
        for n in [0, 1, 3, 100] {
            let hits: Vec<AtomicUsize> = (0..n).map(|_| AtomicUsize::new(0)).collect();
            pool.run(n, &|i| {
                hits[i].fetch_add(1, Ordering::Relaxed);
            });
            assert!(hits.iter().all(|h| h.load(Ordering::Relaxed) == 1));
        }
        let mut out = vec![0usize; 64];
        let sh = Shards::new(&mut out);
        pool.run(8, &|i| unsafe { sh.part(i * 8, 8) }.iter_mut().for_each(|x| *x = i));
        assert_eq!(out[63], 7);
    }
}
