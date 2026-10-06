//! Cooperative lock acquisition for cancellable disk operations.
use std::{
    fs::File,
    io,
    sync::{Mutex, MutexGuard, TryLockError},
    thread,
    time::Duration,
};

fn check(cancel: &dyn Fn() -> bool) -> io::Result<()> {
    if cancel() {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Opération annulée.",
        ))
    } else {
        Ok(())
    }
}
pub(super) fn mutex<'a, T>(
    lock: &'a Mutex<T>,
    cancel: &dyn Fn() -> bool,
) -> io::Result<MutexGuard<'a, T>> {
    loop {
        check(cancel)?;
        match lock.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(error)) => return Ok(error.into_inner()),
            Err(TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(20)),
        }
    }
}
pub(super) fn file(file: &File, cancel: &dyn Fn() -> bool) -> io::Result<()> {
    loop {
        check(cancel)?;
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(std::fs::TryLockError::Error(error)) => return Err(error),
            Err(std::fs::TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(20)),
        }
    }
}
