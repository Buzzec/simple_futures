//! Simple futures for use in async operations.
#![cfg_attr(not(any(test, feature = "std")), no_std)]
#![warn(missing_docs, missing_debug_implementations, unused_import_braces)]

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod atomic_state;
#[cfg(feature = "alloc")]
pub mod complete_future;
#[cfg(feature = "alloc")]
pub mod race_future;
#[cfg(feature = "alloc")]
pub mod value_future;

trait EnsureSend: Send {}
trait EnsureSync: Sync {}

#[cfg(test)]
pub mod test {
    use std::mem::forget;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::task::{RawWaker, RawWakerVTable, Waker};

    #[allow(unsafe_code)]
    static WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(
        |ptr| {
            let ptr_val = unsafe { Arc::from_raw(ptr as *const AtomicUsize) };
            let clone = ptr_val.clone();
            forget(ptr_val);
            RawWaker::new(Arc::into_raw(clone) as *const (), &WAKER_VTABLE)
        },
        |ptr| {
            let ptr_val = unsafe { Arc::from_raw(ptr as *const AtomicUsize) };
            ptr_val.fetch_add(1, Ordering::SeqCst);
            drop(ptr_val);
        },
        |ptr| {
            unsafe { &*(ptr as *const AtomicUsize) }.fetch_add(1, Ordering::SeqCst);
        },
        |ptr| {
            let ptr_val = unsafe { Arc::from_raw(ptr as *const AtomicUsize) };
            drop(ptr_val);
        },
    );

    #[allow(unsafe_code)]
    pub fn get_waker(wake_count: Arc<AtomicUsize>) -> Waker {
        unsafe {
            Waker::from_raw(RawWaker::new(
                Arc::into_raw(wake_count) as *const (),
                &WAKER_VTABLE,
            ))
        }
    }
}
