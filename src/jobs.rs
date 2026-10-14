//! Work off the frame: classifiers train, index, judge and read texts on threads of
//! their own, so the window never waits for them. A job runs a closure on a thread and
//! hands back what to do with its result, which `apply_done` runs on the main world.
//! Results from a save that's been closed since are dropped.

use bevy::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use anend_core::classify::Pool;

/// what a finished job does to the world
pub type Then = Box<dyn FnOnce(&mut World) + Send>;

#[derive(Resource)]
pub struct Work {
    tx: Sender<(u64, Then)>,
    rx: Mutex<Receiver<(u64, Then)>>,
    /// bumped when the save changes, so late results are dropped
    generation: Arc<AtomicU64>,
    /// raised for every running job when the save changes
    stop: Arc<AtomicBool>,
}

impl Default for Work {
    fn default() -> Self {
        let (tx, rx) = channel();
        Work {
            tx,
            rx: Mutex::new(rx),
            generation: Arc::new(AtomicU64::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Work {
    /// runs `job` on a thread of its own; what it returns runs on the world after
    pub fn spawn(&self, name: &str, job: impl FnOnce() -> Then + Send + 'static) {
        let tx = self.tx.clone();
        let generation = self.generation.load(Ordering::SeqCst);
        let spawned = std::thread::Builder::new().name(format!("anend {}", name)).spawn(move || {
            let then = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)) {
                Ok(t) => t,
                Err(_) => Box::new(|w: &mut World| {
                    let mut status = w.resource_mut::<crate::ui::Status>();
                    status.text = "something went wrong in the background".into();
                    status.until = f64::MAX;
                }),
            };
            let _ = tx.send((generation, then));
        });
        if let Err(e) = spawned {
            warn!("couldn't start a thread: {}", e);
        }
    }

    /// a flag the long jobs check, raised when the save changes
    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        self.stop.clone()
    }

    /// the save changed: running jobs stop and their results are dropped
    pub fn new_save(&mut self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.stop.store(true, Ordering::SeqCst);
        self.stop = Arc::new(AtomicBool::new(false));
    }
}

/// runs what finished jobs left behind
pub fn apply_done(world: &mut World) {
    let done: Vec<(u64, Then)> = {
        let work = world.resource::<Work>();
        let rx = work.rx.lock().unwrap_or_else(|e| e.into_inner());
        rx.try_iter().collect()
    };
    if done.is_empty() {
        return;
    }
    let now = world.resource::<Work>().generation.load(Ordering::SeqCst);
    for (generation, then) in done {
        if generation == now {
            then(world);
        }
    }
}

/// a pool for one job's inference, leaving a thread for the window
pub fn pool() -> Pool {
    Pool::new(anend_core::classify::pool::available().saturating_sub(1).max(1))
}

pub fn debug_wheel(
    mut scroll: MessageReader<bevy::picking::events::PointerScroll>,
    q: Query<(&ComputedNode, &Node)>,
    parents: Query<&ChildOf>,
) {
    for s in scroll.read() {
        for e in std::iter::once(s.entity).chain(parents.iter_ancestors(s.entity)) {
            if let Ok((c, n)) = q.get(e) {
                info!("ANC {:?} size={:?} dir={:?} grow={} minh={:?} h={:?} ov={:?}", e, c.size(), n.flex_direction, n.flex_grow, n.min_height, n.height, n.overflow);
            }
        }
        break;
    }
}
