//! Locks that outlive a panic.

use std::sync::{Mutex, MutexGuard};

/// Lock `m`, taking it back if a thread panicked while holding it.
///
/// A poisoned mutex only flags that a holder panicked; the data is still
/// there. Every lock taken through this guards a value that one `push`,
/// `take` or store leaves whole, so carrying on is sound, and failing
/// instead would end the thread (on the UI thread, the app). The poison
/// is cleared so `site` is logged once per panic, not on every lock.
pub fn lock_recover<'a, T>(m: &'a Mutex<T>, site: &str) -> MutexGuard<'a, T> {
    m.lock().unwrap_or_else(|poisoned| {
        log::warn!("{site}: lock was poisoned by a panic, recovering");
        m.clear_poison();
        poisoned.into_inner()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_poisoned_lock_gives_back_its_data_and_is_cleared() {
        let m = Arc::new(Mutex::new(vec![1]));
        let held = Arc::clone(&m);
        let _ = std::thread::spawn(move || {
            let mut guard = held.lock().unwrap();
            guard.push(2);
            panic!("poison the lock");
        })
        .join();
        assert!(m.is_poisoned());
        assert_eq!(*lock_recover(&m, "test"), [1, 2]);
        assert!(m.lock().is_ok());
    }
}
