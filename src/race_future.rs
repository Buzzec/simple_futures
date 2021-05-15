//! A Future that races two futures and returns the first one to finish.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering, AtomicU8};
use bitflags::bitflags;
use core::future::Future;
use core::task::{Context, Poll, Waker, RawWakerVTable, RawWaker};
use core::pin::Pin;
use atomic_swapping::option::AtomicSwapOption;
use core::mem::forget;

/// A Future that races two futures and returns the first one to finish.
#[derive(Debug)]
pub struct RaceFuture<OkF, ErrF> {
    ok_future: OkF,
    err_future: ErrF,
    inner: Arc<RaceFutureInner>,
}
impl<OkF, ErrF> RaceFuture<OkF, ErrF>{
    fn priv_new(ok_future: OkF, err_future: ErrF, should_finish: Option<Arc<AtomicBool>>) -> Self{
        Self{
            ok_future,
            err_future,
            inner: Arc::new(RaceFutureInner {
                state: AtomicU8::new(RaceFutureState::OK_READY.bits | RaceFutureState::ERR_READY.bits),
                waker: AtomicSwapOption::from_none(),
                should_finish,
            })
        }
    }
    /// Creates a new [`RaceFuture`] from two futures. Look to [`RaceFuture::should_finish`] if these futures should know about each others' completions.
    pub fn new(ok_future: OkF, err_future: ErrF) -> Self {
        Self::priv_new(ok_future, err_future, None)
    }
    /// Creates a new [`RaceFuture`] from two future generator functions. These generator functions take a [`ShouldFinish`] that tells them when the other future has finished.
    pub fn should_finish(ok_future_func: impl FnOnce(ShouldFinish) -> OkF, err_future_func: impl FnOnce(ShouldFinish) -> ErrF) -> Self{
        let should_finish = Arc::new(AtomicBool::new(false));
        Self::priv_new(ok_future_func(ShouldFinish(should_finish.clone())), err_future_func(ShouldFinish(should_finish.clone())), Some(should_finish))
    }
    /// Creates a new [`RaceFuture`] from a future generator function for Ok. These generator functions take a [`ShouldFinish`] that tells them when the other future has finished.
    pub fn should_finish_ok(ok_future_func: impl FnOnce(ShouldFinish) -> OkF, err_future: ErrF) -> Self{
        let should_finish = Arc::new(AtomicBool::new(false));
        Self::priv_new(ok_future_func(ShouldFinish(should_finish.clone())), err_future, Some(should_finish))
    }
    /// Creates a new [`RaceFuture`] from a future generator function for Err. These generator functions take a [`ShouldFinish`] that tells them when the other future has finished.
    pub fn should_finish_err(ok_future: OkF, err_future_func: impl FnOnce(ShouldFinish) -> ErrF) -> Self{
        let should_finish = Arc::new(AtomicBool::new(false));
        Self::priv_new(ok_future, err_future_func(ShouldFinish(should_finish.clone())), Some(should_finish))
    }
}
impl<OkF, ErrF> Future for RaceFuture<OkF, ErrF> where OkF: Future, ErrF: Future{
    type Output = Result<OkF::Output, ErrF::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Safety: safe because self will not move in this function
        let self_ref = unsafe{ self.get_unchecked_mut() };
        let mut state = self_ref.inner.state.swap(RaceFutureState::POLLING.bits, Ordering::AcqRel);
        debug_assert_eq!(state & RaceFutureState::POLLING.bits, 0);
        loop{
            self_ref.inner.waker.set(Some(cx.waker().clone()));
            if state & RaceFutureState::OK_READY.bits == RaceFutureState::OK_READY.bits{
                if let Poll::Ready(value) = unsafe { Pin::new_unchecked(&mut self_ref.ok_future).poll(&mut Context::from_waker(&Waker::from_raw(self_ref.inner.clone().into_raw_ok_waker()))) }{
                    if let Some(should_finish) = &self_ref.inner.should_finish{
                        should_finish.store(true, Ordering::Release);
                    }
                    return Poll::Ready(Ok(value));
                }
            }
            if state & RaceFutureState::ERR_READY.bits == RaceFutureState::ERR_READY.bits{
                if let Poll::Ready(value) = unsafe { Pin::new_unchecked(&mut self_ref.err_future).poll(&mut Context::from_waker(&Waker::from_raw(self_ref.inner.clone().into_raw_err_waker()))) }{
                    if let Some(should_finish) = &self_ref.inner.should_finish{
                        should_finish.store(true, Ordering::Release);
                    }
                    return Poll::Ready(Err(value));
                }
            }
            match self_ref.inner.state.compare_exchange(RaceFutureState::POLLING.bits, RaceFutureState::WAITING.bits, Ordering::AcqRel, Ordering::Acquire){
                Ok(_) => return Poll::Pending,
                Err(new_state) => state = new_state,
            }
        }
    }
}

#[derive(Debug)]
struct RaceFutureInner{
    state: AtomicU8,
    waker: AtomicSwapOption<Waker>,
    should_finish: Option<Arc<AtomicBool>>,
}
impl RaceFutureInner{
    fn into_raw_ok_waker(self: Arc<Self>) -> RawWaker{
        RawWaker::new(Arc::into_raw(self) as *const (), &OK_FUTURE_VTABLE)
    }

    fn into_raw_err_waker(self: Arc<Self>) -> RawWaker{
        RawWaker::new(Arc::into_raw(self) as *const (), &ERR_FUTURE_VTABLE)
    }
}

/// Tells when this future should finish.
#[derive(Clone, Debug)]
pub struct ShouldFinish(Arc<AtomicBool>);
impl ShouldFinish {
    /// Returns [`true`] if this future should finish.
    pub fn should_finish(&self) -> bool{
        self.0.load(Ordering::Acquire)
    }
}

bitflags!{
    struct RaceFutureState: u8{
        const WAITING   = 0b00000000;
        const OK_READY  = 0b00000001;
        const ERR_READY = 0b00000010;
        const POLLING   = 0b00000100;
    }
}

static OK_FUTURE_VTABLE: RawWakerVTable = RawWakerVTable::new(
    |ptr|unsafe {
        let inner = get_inner_from_ptr(ptr);
        let out = inner.clone();
        forget(inner);
        out.into_raw_ok_waker()
    },
    |ptr|unsafe{
        let inner = get_inner_from_ptr(ptr);
        let state = inner.state.fetch_and(RaceFutureState::OK_READY.bits, Ordering::AcqRel);
        if state & (RaceFutureState::ERR_READY.bits | RaceFutureState::POLLING.bits) == 0{
            if let Some(waker) = inner.waker.take(){
                waker.wake();
            }
        }
    },
    |ptr|unsafe{
        let inner = get_inner_from_ptr(ptr);
        let state = inner.state.fetch_and(RaceFutureState::OK_READY.bits, Ordering::AcqRel);
        if state & (RaceFutureState::ERR_READY.bits | RaceFutureState::POLLING.bits) == 0{
            if let Some(waker) = inner.waker.take(){
                waker.wake();
            }
        }
        forget(inner)
    },
    |ptr|unsafe{
        drop(get_inner_from_ptr(ptr));
    },
);
static ERR_FUTURE_VTABLE: RawWakerVTable = RawWakerVTable::new(
    |ptr|unsafe {
        let inner = get_inner_from_ptr(ptr);
        let out = inner.clone();
        forget(inner);
        out.into_raw_err_waker()
    },
    |ptr|unsafe{
        let inner = get_inner_from_ptr(ptr);
        let state = inner.state.fetch_and(RaceFutureState::ERR_READY.bits, Ordering::AcqRel);
        if state & (RaceFutureState::OK_READY.bits | RaceFutureState::POLLING.bits) == 0{
            if let Some(waker) = inner.waker.take(){
                waker.wake();
            }
        }
    },
    |ptr|unsafe{
        let inner = get_inner_from_ptr(ptr);
        let state = inner.state.fetch_and(RaceFutureState::ERR_READY.bits, Ordering::AcqRel);
        if state & (RaceFutureState::OK_READY.bits | RaceFutureState::POLLING.bits) == 0{
            if let Some(waker) = inner.waker.take(){
                waker.wake();
            }
        }
        forget(inner)
    },
    |ptr|unsafe{
        drop(get_inner_from_ptr(ptr));
    },
);

unsafe fn get_inner_from_ptr(ptr: *const ()) -> Arc<RaceFutureInner>{
    Arc::from_raw(ptr as *const RaceFutureInner)
}

