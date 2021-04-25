//! A future that can be assigned once and returns a value `T`.

use alloc::sync::{Arc, Weak};
use atomic_swapping::AtomicSwap;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

/// A Future that returns a `T` and can be assigned from inner
#[derive(Debug)]
pub struct ValueFuture<T> {
    inner: Arc<AtomicSwap<ValueFutureInner<T>>>,
}
impl<T> ValueFuture<T> {
    /// Creates a new `ValueFuture`
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AtomicSwap::new(ValueFutureInner {
                waker: None,
                data: None,
            })),
        }
    }

    /// Gets a handle to this future meant to be sent to the worker.
    pub fn get_handle(&self) -> ValueFutureHandle<T> {
        ValueFutureHandle {
            inner: Arc::downgrade(&self.inner),
        }
    }

    /// Completes this future returning the last state of the future. If this is
    /// [`Err`] it is usually an error.
    pub fn assign(&self, val: T) -> Result<(), T> {
        match self.inner.swap(ValueFutureInner {
            waker: None,
            data: Some(val),
        }) {
            ValueFutureInner {
                data: Some(old), ..
            } => Err(old),
            ValueFutureInner {
                waker: Some(waker), ..
            } => {
                waker.wake();
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
impl<T> Default for ValueFuture<T> {
    fn default() -> Self {
        Self::new()
    }
}
impl<T> Future for ValueFuture<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self
            .inner
            .swap(ValueFutureInner {
                waker: Some(cx.waker().clone()),
                data: None,
            })
            .data
        {
            Some(data) => Poll::Ready(data),
            None => Poll::Pending,
        }
    }
}

/// A handle for [`ValueFuture`]. Meant to be sent where the work
/// is actually getting done.
#[derive(Debug)]
pub struct ValueFutureHandle<T> {
    inner: Weak<AtomicSwap<ValueFutureInner<T>>>,
}
impl<T> ValueFutureHandle<T> {
    /// Assigns a value to the future. Returns `Err` if this has already been
    /// assigned to.
    pub fn assign(&self, val: T) -> Option<Result<(), T>> {
        match self.inner.upgrade() {
            None => None,
            Some(inner) => {
                match inner.swap(ValueFutureInner {
                    waker: None,
                    data: Some(val),
                }) {
                    ValueFutureInner {
                        data: Some(old), ..
                    } => Some(Err(old)),
                    ValueFutureInner {
                        waker: Some(waker), ..
                    } => {
                        waker.wake();
                        Some(Ok(()))
                    }
                    _ => Some(Ok(())),
                }
            }
        }
    }
}
#[derive(Debug)]
struct ValueFutureInner<T> {
    waker: Option<Waker>,
    data: Option<T>,
}

#[cfg(test)]
mod test {
    use crate::test::get_waker;
    use crate::value_future::ValueFuture;
    use rand::{thread_rng, Rng};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::task::{Context, Poll};

    #[test]
    fn functionality_test() {
        for _ in 0..1024 {
            let mut value_future = ValueFuture::new();
            let wake_count = Arc::new(AtomicUsize::new(0));
            assert!(Pin::new(&mut value_future)
                .poll(&mut Context::from_waker(&get_waker(wake_count.clone())))
                .is_pending());
            let value: usize = thread_rng().gen();
            assert!(value_future.assign(value).is_ok());
            assert_eq!(1, wake_count.load(Ordering::SeqCst));
            let result = Pin::new(&mut value_future)
                .poll(&mut Context::from_waker(&get_waker(wake_count.clone())));
            match result {
                Poll::Ready(future_value) => assert_eq!(future_value, value),
                Poll::Pending => panic!("Result should be ready!"),
            }
        }
    }
}
