//! Starting the connectors' threads, and taking the locks they share. A
//! lock whose holder panicked is still taken, so one failed thread does not
//! stop the others.

use std::sync::{LockResult, Mutex, MutexGuard, PoisonError};
use std::thread;

use esp_idf_svc::sys::{self, EspError};

/// Runs `work` on a new thread called `name` with a stack of `stack_bytes`.
/// A thread that cannot start is out of memory.
pub(crate) fn spawn(
    name: &str,
    stack_bytes: usize,
    work: impl FnOnce() + Send + 'static,
) -> Result<(), EspError> {
    thread::Builder::new()
        .name(name.to_owned())
        .stack_size(stack_bytes)
        .spawn(work)
        .map(drop)
        .map_err(|_| EspError::from_infallible::<{ sys::ESP_ERR_NO_MEM }>())
}

/// Takes `mutex`, whether or not its last holder panicked.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    unpoisoned(mutex.lock())
}

/// The guard of a lock or a wait, whether or not its holder panicked.
pub(crate) fn unpoisoned<T>(result: LockResult<T>) -> T {
    result.unwrap_or_else(PoisonError::into_inner)
}
