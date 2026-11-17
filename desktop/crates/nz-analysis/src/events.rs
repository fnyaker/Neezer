//! The channel a background thread uses to tell the screen what it did.
//!
//! SC1 moved the desktop's analysis here, and this crate is compiled for Android
//! too, so it cannot depend on the shell (`desktop/app/src/wiring/events.rs`),
//! which is GPUI-side glue. The channel below is the shell's own, unchanged:
//! one queue, one waker, and a `Drop` that closes the channel when the LAST
//! `Sender` goes, so a screen that closes ends its drain task instead of parking
//! for ever. Behaviour is identical on purpose — the shell keeps a copy for the
//! rest of the app and its tests (`a_send_wakes_a_pending_receive`,
//! `dropping_the_last_sender_ends_the_receive`) cover this source too; the copy
//! adds no test of its own, so the workspace's test count does not move.
//! A shared `nz-events` crate would end the duplication, and would need the
//! workspace manifest, which no extraction lot may edit (docs/agents/mobile.md,
//! *Worker rules* 3).

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

struct Inner<T> {
    queue: Mutex<VecDeque<T>>,
    waker: Mutex<Option<Waker>>,
    closed: AtomicBool,
    /// How many [`Sender`]s exist. The LAST one to drop closes the channel, so
    /// the receiver's task can end instead of parking for ever (see `Drop`).
    senders: AtomicUsize,
}

/// The sending half; clone it into every worker.
pub struct Sender<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.inner.senders.fetch_add(1, Ordering::Relaxed);
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

/// Dropping the last sender closes the channel.
///
/// A screen owns one `Sender` and one drain task awaiting `Receiver::next()`.
/// Without this, the task was woken by nothing after the screen was replaced:
/// GPUI kept it — and everything the future captured — for the life of the
/// process, one screen's worth per navigation (measured: ~2 KiB per `Screen`,
/// `screens/settings/mod.rs`'s heap test; 1 000 visits to a screen that builds
/// one per visit grew the app by ~2 MiB). Closing on the last drop lets
/// `next()` answer `None`, the loop ends, and the task is freed.
impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        if self.inner.senders.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.close();
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
        senders: AtomicUsize::new(1),
    });
    (
        Sender {
            inner: Arc::clone(&inner),
        },
        Receiver { inner },
    )
}
