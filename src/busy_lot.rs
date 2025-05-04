use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Shared, lock-free counter of active jobs.
/// Cloning is cheap (Arc); dropping a ticket auto-decrements.
#[derive(Clone)]
pub struct BusyLot {
    inner: Arc<AtomicUsize>,
}

/// RAII ticket returned from `BusyLot::park()`.
/// When the ticket is dropped (even on panic) the lot counter goes down.
pub struct Ticket {
    lot: BusyLot,
}

impl BusyLot {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Takes a parking space and returns a ticket.
    pub fn park(&self) -> Ticket {
        self.inner.fetch_add(1, Ordering::AcqRel);
        Ticket { lot: self.clone() }
    }

    /// `true` if at least one ticket is still parked.
    pub fn is_busy(&self) -> bool {
        self.inner.load(Ordering::Acquire) != 0
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        self.lot.inner.fetch_sub(1, Ordering::AcqRel);
    }
}
