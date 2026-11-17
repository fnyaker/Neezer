//! The one way a background thread tells the UI something.
//!
//! GPUI's `App` is single-threaded: the audio engine, the tokio runtime, the
//! MPRIS thread and the D-Bus name all live elsewhere, so each of them sends a
//! [`crate::state::AppEvent`] through this channel and the shell drains it in a
//! foreground task. It is a real channel, not a poll: [`Sender::send`] wakes the
//! pending receiver immediately, which keeps an event ordering ("the track
//! started" before "the bar moved") without a timer in between.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

struct Inner<T> {
    queue: Mutex<VecDeque<T>>,
    waker: Mutex<Option<Waker>>,
    closed: AtomicBool,
}

/// The sending half; clone it into every worker.
pub struct Sender<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> Sender<T> {
    /// Never blocks, never fails while a receiver exists. A send after the app
    /// is gone is dropped, which is exactly what a worker shutting down wants.
    pub fn send(&self, value: T) {
        let mut queue = self
            .inner
            .queue
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        queue.push_back(value);
        drop(queue);
        if let Some(waker) = self
            .inner
            .waker
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take()
        {
            waker.wake();
        }
    }

    /// No receiver will ever read again.
    pub fn close(&self) {
        self.inner.closed.store(true, Ordering::Release);
        if let Some(waker) = self
            .inner
            .waker
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take()
        {
            waker.wake();
        }
    }
}

/// The receiving half, owned by the shell. `next()` is a future that resolves on
/// the next message, so the shell's task sleeps instead of polling.
pub struct Receiver<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Receiver<T> {
    pub fn next(&self) -> Next<'_, T> {
        Next { receiver: self }
    }
}

/// One pending receive.
pub struct Next<'a, T> {
    receiver: &'a Receiver<T>,
}

impl<T> Future for Next<'_, T> {
    type Output = Option<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let inner = &self.receiver.inner;
        if let Some(value) = inner
            .queue
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .pop_front()
        {
            return Poll::Ready(Some(value));
        }
        if inner.closed.load(Ordering::Acquire) {
            return Poll::Ready(None);
        }
        // Register BEFORE re-checking the queue: a send that lands between the
        // first pop and this store would otherwise be lost with no waker.
        *inner.waker.lock().unwrap_or_else(|err| err.into_inner()) = Some(cx.waker().clone());
        if let Some(value) = inner
            .queue
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .pop_front()
        {
            return Poll::Ready(Some(value));
        }
        if inner.closed.load(Ordering::Acquire) {
            return Poll::Ready(None);
        }
        Poll::Pending
    }
}

/// A channel and its receiver.
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let inner = Arc::new(Inner {
        queue: Mutex::new(VecDeque::new()),
        waker: Mutex::new(None),
        closed: AtomicBool::new(false),
    });
    (
        Sender {
            inner: Arc::clone(&inner),
        },
        Receiver { inner },
    )
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::task::Wake;

    use super::*;

    struct CountingWaker(AtomicUsize);

    impl Wake for CountingWaker {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn a_send_wakes_a_pending_receive() {
        let (tx, rx) = channel::<u32>();
        let counter = Arc::new(CountingWaker(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&counter));
        let mut context = Context::from_waker(&waker);
        let mut next = Box::pin(rx.next());
        assert!(matches!(next.as_mut().poll(&mut context), Poll::Pending));
        tx.send(7);
        assert_eq!(counter.0.load(Ordering::SeqCst), 1);
        assert_eq!(next.as_mut().poll(&mut context), Poll::Ready(Some(7)));
    }

    #[test]
    fn closing_drains_what_is_queued_then_ends() {
        let (tx, rx) = channel::<u8>();
        tx.send(1);
        tx.close();
        let counter = Arc::new(CountingWaker(AtomicUsize::new(0)));
        let waker = Waker::from(counter);
        let mut context = Context::from_waker(&waker);
        let mut next = Box::pin(rx.next());
        assert_eq!(next.as_mut().poll(&mut context), Poll::Ready(Some(1)));
        assert_eq!(next.as_mut().poll(&mut context), Poll::Ready(None));
    }
}
