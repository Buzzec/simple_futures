//! A future that only shows completion and no value, effectively outputting [`()`].
//! ```
//! # use std::sync::atomic::{AtomicUsize, Ordering};
//! # use std::task::Context;
//! # use std::pin::Pin;
//! # use std::sync::Arc;
//! # use std::future::Future;
//! use simple_futures::complete_future::CompleteFuture;
//!
//! let complete_future = CompleteFuture::new();
//! let handle = complete_future.get_handle();
//! // Async
//! let future = async move{
//!     complete_future.await;
//!     println!("Future done!")
//! };
//! // In other thread
//! assert!(!handle.complete().unwrap());
//! ```

use alloc::sync::{Arc, Weak};
use atomic_swapping::AtomicSwap;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use crate::{EnsureSend, EnsureSync};

/// A future that only completes without returning a value
#[derive(Debug)]
pub struct CompleteFuture {
    inner: Arc<AtomicSwap<CompleteFutureInner>>,
}
impl CompleteFuture {
    /// Creates a new complete future
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AtomicSwap::new(CompleteFutureInner {
                waker: None,
                finished: false,
            })),
        }
    }

    /// Gets a handle to this future meant to be sent to the worker.
    pub fn get_handle(&self) -> CompleteFutureHandle {
        CompleteFutureHandle {
            inner: Arc::downgrade(&self.inner),
        }
    }

    /// Completes this future returning the last state of the future. If this is
    /// true it is usually an error.
    pub fn complete(&self) -> bool {
        let inner = self.inner.swap(CompleteFutureInner {
            waker: None,
            finished: true,
        });
        match inner {
            CompleteFutureInner { finished: true, .. } => true,
            CompleteFutureInner {
                waker: Some(waker), ..
            } => {
                waker.wake();
                false
            }
            _ => false,
        }
    }
}
impl Default for CompleteFuture {
    fn default() -> Self {
        Self::new()
    }
}
impl Future for CompleteFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let inner = self.inner.swap(CompleteFutureInner {
            waker: Some(cx.waker().clone()),
            finished: false,
        });
        match inner.finished {
            true => Poll::Ready(()),
            false => Poll::Pending,
        }
    }
}
impl EnsureSend for CompleteFuture{}
impl EnsureSync for CompleteFuture{}

/// A completion handle for [`CompleteFuture`]. Meant to be sent where the work
/// is actually getting done.
#[derive(Debug)]
pub struct CompleteFutureHandle {
    inner: Weak<AtomicSwap<CompleteFutureInner>>,
}
impl CompleteFutureHandle {
    /// Sets the future to complete and wakes the waker if wasn't already
    /// complete. Returns the last state of complete, it is usually an error
    /// if this is true. If returns [`None`] that means the future this is a
    /// handle for was dropped.
    pub fn complete(&self) -> Option<bool> {
        match self.inner.upgrade() {
            None => None,
            Some(inner) => {
                let inner = inner.swap(CompleteFutureInner {
                    waker: None,
                    finished: true,
                });
                match inner {
                    CompleteFutureInner { finished: true, .. } => Some(true),
                    CompleteFutureInner {
                        waker: Some(waker), ..
                    } => {
                        waker.wake();
                        Some(false)
                    }
                    _ => Some(false),
                }
            }
        }
    }
}
impl EnsureSend for CompleteFutureHandle{}
impl EnsureSync for CompleteFutureHandle{}

/// The inside of a complete future, intended to be passed to the function doing
/// the work
#[derive(Debug)]
struct CompleteFutureInner {
    waker: Option<Waker>,
    finished: bool,
}

#[cfg(test)]
mod test{
    use crate::complete_future::CompleteFuture;
    use core::future::Future;
    use core::pin::Pin;
    use core::task::Context;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use alloc::sync::Arc;
    use crate::test::get_waker;

    #[test]
    fn functionality_test(){
        let mut complete_future = CompleteFuture::new();
        let wake_count = Arc::new(AtomicUsize::new(0));
        assert!(Pin::new(&mut complete_future).poll(&mut Context::from_waker(&get_waker(wake_count.clone()))).is_pending());
        assert!(!complete_future.complete());
        assert_eq!(1, wake_count.load(Ordering::SeqCst));
        assert!(Pin::new(&mut complete_future).poll(&mut Context::from_waker(&get_waker(wake_count.clone()))).is_ready());
    }
}
