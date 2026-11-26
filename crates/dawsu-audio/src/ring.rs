//! Bounded lock-free SPSC ring buffer and cache-line padding for real-time audio queues.

use std::cell::UnsafeCell;
use std::fmt;
use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Aligns cross-thread atomics to 128 bytes so Apple Silicon (128 B lines) and x86 (64 B lines)
/// never suffer false sharing between producer and consumer cursors or telemetry fields.
#[repr(align(128))]
#[derive(Default)]
pub struct CachePadded<T>(pub T);

impl<T> CachePadded<T> {
    #[inline]
    pub const fn new(value: T) -> Self {
        Self(value)
    }

    #[inline]
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for CachePadded<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for CachePadded<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: fmt::Debug> fmt::Debug for CachePadded<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

struct RingBuffer<T, const N: usize> {
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,
    slots: [UnsafeCell<MaybeUninit<T>>; N],
}

impl<T, const N: usize> Drop for RingBuffer<T, N> {
    fn drop(&mut self) {
        let mut tail = *self.tail.get_mut();
        let head = *self.head.get_mut();
        while tail != head {
            let slot_idx = tail & (N - 1);
            // SAFETY: Only slots in `[tail, head)` hold initialized values, and `Drop` has
            // exclusive access via `Arc`.
            unsafe { self.slots[slot_idx].get_mut().assume_init_drop() };
            tail = tail.wrapping_add(1);
        }
    }
}

/// Producer endpoint of a bounded lock-free SPSC ring (`Send`, `!Sync` — wrap in `Mutex` for
/// shared non-RT senders).
pub struct SpscProducer<T, const N: usize> {
    buf: Arc<RingBuffer<T, N>>,
}

/// Consumer endpoint of a bounded lock-free SPSC ring (`Send`, `!Sync` — owned exclusively by
/// the audio callback thread).
pub struct SpscConsumer<T, const N: usize> {
    buf: Arc<RingBuffer<T, N>>,
}

// SAFETY: Ownership transfers through the ring with Acquire/Release synchronization; only one
// producer and one consumer can exist and neither implements `Sync` (`&mut self` operations only).
unsafe impl<T: Send, const N: usize> Send for SpscProducer<T, N> {}
unsafe impl<T: Send, const N: usize> Send for SpscConsumer<T, N> {}

impl<T, const N: usize> fmt::Debug for SpscProducer<T, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpscProducer")
            .field("capacity", &N)
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

impl<T, const N: usize> fmt::Debug for SpscConsumer<T, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpscConsumer")
            .field("capacity", &N)
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

/// Creates a bounded lock-free SPSC ring with power-of-two capacity `N >= 2`.
pub fn spsc_ring<T, const N: usize>() -> (SpscProducer<T, N>, SpscConsumer<T, N>) {
    const {
        assert!(N >= 2 && N.is_power_of_two(), "SPSC ring capacity must be a power of two >= 2");
        assert!(N <= (isize::MAX as usize), "SPSC ring capacity exceeds half usize::MAX");
    }
    let buf = Arc::new(RingBuffer {
        head: CachePadded::new(AtomicUsize::new(0)),
        tail: CachePadded::new(AtomicUsize::new(0)),
        slots: [const { UnsafeCell::new(MaybeUninit::uninit()) }; N],
    });
    (
        SpscProducer { buf: Arc::clone(&buf) },
        SpscConsumer { buf },
    )
}

impl<T, const N: usize> SpscProducer<T, N> {
    #[inline]
    pub const fn capacity(&self) -> usize {
        N
    }

    #[inline]
    pub fn len(&self) -> usize {
        let head = self.buf.head.load(Ordering::Relaxed);
        let tail = self.buf.tail.load(Ordering::Acquire);
        head.wrapping_sub(tail)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Pushes `value` without blocking or allocating; returns `Err(value)` if full.
    #[inline]
    pub fn try_push(&mut self, value: T) -> Result<(), T> {
        let head = self.buf.head.load(Ordering::Relaxed);
        let tail = self.buf.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) >= N {
            return Err(value);
        }
        let slot_idx = head & (N - 1);
        // SAFETY: `head - tail < N`, so `slot_idx` is not currently readable by the consumer,
        // and `&mut self` guarantees no other producer exists.
        unsafe { (*self.buf.slots[slot_idx].get()).write(value) };
        self.buf.head.store(head.wrapping_add(1), Ordering::Release);
        Ok(())
    }
}

impl<T, const N: usize> SpscConsumer<T, N> {
    #[inline]
    pub const fn capacity(&self) -> usize {
        N
    }

    #[inline]
    pub fn len(&self) -> usize {
        let head = self.buf.head.load(Ordering::Acquire);
        let tail = self.buf.tail.load(Ordering::Relaxed);
        head.wrapping_sub(tail)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Pops the next item without blocking or allocating; returns `None` if empty.
    #[inline]
    pub fn try_pop(&mut self) -> Option<T> {
        let tail = self.buf.tail.load(Ordering::Relaxed);
        let head = self.buf.head.load(Ordering::Acquire);
        if tail == head {
            return None;
        }
        let slot_idx = tail & (N - 1);
        // SAFETY: `tail != head` with Acquire load on `head` guarantees the slot was initialized
        // before Release store on `head`, and `&mut self` guarantees no other consumer exists.
        let value = unsafe { (*self.buf.slots[slot_idx].get()).assume_init_read() };
        self.buf.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn fifo_wraparound_and_full_rejection() {
        let (mut tx, mut rx) = spsc_ring::<usize, 4>();
        assert!(rx.is_empty());
        assert_eq!(tx.try_push(10), Ok(()));
        assert_eq!(tx.try_push(20), Ok(()));
        assert_eq!(tx.try_push(30), Ok(()));
        assert_eq!(tx.try_push(40), Ok(()));
        assert_eq!(tx.try_push(50), Err(50));
        assert_eq!(rx.len(), 4);
        assert_eq!(rx.try_pop(), Some(10));
        assert_eq!(rx.try_pop(), Some(20));
        assert_eq!(tx.try_push(50), Ok(()));
        assert_eq!(tx.try_push(60), Ok(()));
        assert_eq!(tx.try_push(70), Err(70));
        assert_eq!(rx.try_pop(), Some(30));
        assert_eq!(rx.try_pop(), Some(40));
        assert_eq!(rx.try_pop(), Some(50));
        assert_eq!(rx.try_pop(), Some(60));
        assert_eq!(rx.try_pop(), None);
    }

    #[test]
    fn drop_cleans_up_unconsumed_elements() {
        struct DropCount(Arc<AtomicUsize>);
        impl Drop for DropCount {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let dropped = Arc::new(AtomicUsize::new(0));
        {
            let (mut tx, mut rx) = spsc_ring::<DropCount, 8>();
            for _ in 0..5 {
                assert!(tx.try_push(DropCount(dropped.clone())).is_ok());
            }
            assert!(rx.try_pop().is_some());
            assert_eq!(dropped.load(Ordering::Relaxed), 1);
        }
        assert_eq!(dropped.load(Ordering::Relaxed), 5);
    }

    #[test]
    fn concurrent_spsc_preserves_order_across_ten_thousand_items() {
        let (mut tx, mut rx) = spsc_ring::<usize, 64>();
        let total = 10_000;
        let producer = std::thread::spawn(move || {
            for i in 0..total {
                let mut v = i;
                while let Err(rejected) = tx.try_push(v) {
                    v = rejected;
                    std::thread::yield_now();
                }
            }
        });
        let mut received = Vec::with_capacity(total);
        while received.len() < total {
            if let Some(v) = rx.try_pop() {
                received.push(v);
            } else {
                std::thread::yield_now();
            }
        }
        producer.join().unwrap();
        let expected: Vec<usize> = (0..total).collect();
        assert_eq!(received, expected);
    }
}
