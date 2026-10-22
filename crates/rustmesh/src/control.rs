//! Scoped resource limits and cancellation for meshing calls.
//!
//! [`with_control`] sets limits for the meshing calls made inside it. The scope
//! is thread-local and restored even on unwinding, so it does not affect
//! meshing on other threads. Cell limits count the working complex, including
//! exterior cells, so they are conservative. Memory is not a limit: a caller
//! that needs one must run meshing in a separate process.
use std::{
    cell::RefCell,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum StopReason {
    Converged,
    NodeBudget,
    CellBudget,
    PassBudget,
    TimeBudget,
    MemoryLimit,
    Cancelled,
    Stalled,
    NoPlaceableVertex,
    GeometryRecoveryFailed,
    BackendFailure,
    OutputBudget,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Limits {
    pub max_nodes: usize,
    pub max_cells: usize,
    /// Work iterations, including 2D refinement queue visits and 3D local rounds.
    pub max_iterations: usize,
    pub time_seconds: f64,
    pub max_output_bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_nodes: 100_000,
            max_cells: 2_000_000,
            max_iterations: 5_000_000,
            time_seconds: 120.0,
            max_output_bytes: 256 * 1024 * 1024,
        }
    }
}
impl Limits {
    pub fn validate(&self) -> Result<(), crate::MeshError> {
        if self.max_nodes == 0
            || self.max_cells == 0
            || self.max_iterations == 0
            || !self.time_seconds.is_finite()
            || self.time_seconds <= 0.0
            || self.max_output_bytes == 0
        {
            return Err(crate::MeshError::BadOption(
                "all resource limits must be positive and finite".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct MeshControl {
    pub limits: Limits,
    cancelled: Arc<AtomicBool>,
    iterations: Arc<AtomicUsize>,
    elapsed_seconds: Arc<dyn Fn() -> f64 + Send + Sync>,
    stopped: Arc<Mutex<Option<StopReason>>>,
}
impl MeshControl {
    /// The clock returns elapsed seconds since this job began, not wall-clock time.
    pub fn new(limits: Limits, elapsed_seconds: impl Fn() -> f64 + Send + Sync + 'static) -> Self {
        Self {
            limits,
            cancelled: Arc::new(AtomicBool::new(false)),
            iterations: Arc::new(AtomicUsize::new(0)),
            elapsed_seconds: Arc::new(elapsed_seconds),
            stopped: Arc::new(Mutex::new(None)),
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    pub fn iterations(&self) -> usize {
        self.iterations.load(Ordering::Relaxed)
    }
    pub fn stop_reason(&self) -> Option<StopReason> {
        *self.stopped.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn check(&self, nodes: usize, cells: usize, iteration: bool) -> Result<(), crate::MeshError> {
        let reason = if let Some(reason) = self.stop_reason() {
            Some(reason)
        } else if self.cancelled.load(Ordering::Relaxed) {
            Some(StopReason::Cancelled)
        } else if (self.elapsed_seconds)() >= self.limits.time_seconds {
            Some(StopReason::TimeBudget)
        } else if nodes > self.limits.max_nodes {
            Some(StopReason::NodeBudget)
        } else if cells > self.limits.max_cells {
            Some(StopReason::CellBudget)
        } else if iteration
            && self.iterations.fetch_add(1, Ordering::Relaxed) >= self.limits.max_iterations
        {
            Some(StopReason::PassBudget)
        } else {
            None
        };
        match reason {
            Some(r) => {
                *self.stopped.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
                Err(crate::MeshError::Stopped(r))
            }
            None => Ok(()),
        }
    }
}
thread_local! { static CURRENT: RefCell<Option<MeshControl>> = const { RefCell::new(None) }; }

/// Runs `f` with `control` active: the 2D and 3D meshing calls inside it stop
/// on cancellation or when a limit is passed, and return [`crate::MeshError::Stopped`].
pub fn with_control<T>(control: MeshControl, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<MeshControl>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CURRENT.with(|c| *c.borrow_mut() = self.0.take());
        }
    }
    let previous = CURRENT.with(|c| c.replace(Some(control)));
    let _restore = Restore(previous);
    f()
}

pub(crate) fn checkpoint(
    nodes: usize,
    cells: usize,
    iteration: bool,
) -> Result<(), crate::MeshError> {
    CURRENT.with(|c| match c.borrow().as_ref() {
        Some(control) => control.check(nodes, cells, iteration),
        None => Ok(()),
    })
}
