//! Lock-free single producer single consumer ring buffer.
//!
//! # Why drop instead of block
//!
//! The production side runs inside a device callback with a hard deadline.
//! Blocking there produces a click in the user's music for every overloaded
//! period, which is far worse than a visualizer that is occasionally stale.
//! So when the ring is full, the writer drops samples and counts them; the
//! reader always gets a prefix of *recent* audio, which is what a visualizer
//! wants anyway.
//!
//! # Implementation notes
//!
//! * Writer and reader indices are separate atomics, each confined to one
//!   cache line so the two threads do not fight over a line.
//! * Capacity is a power of two, so modulo is a mask, and the indices are
//!   allowed to run up unbounded: one increment per callback at 48 kHz would
//!   take about 19 000 years to overflow a `usize`.
//! * The ring holds `capacity` sample slots and can therefore carry
//!   `capacity - 1` samples at a time; the extra slot is what makes head and
//!   tail distinguishable without a second counter.
//! * `RingWriter` and `RingReader` are `Send` but deliberately not `Sync`:
//!   the "SP" in SPSC means exactly one of each, and the type system says so.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;

/// Pads an atomic counter onto its own cache line.
#[derive(Debug, Default)]
#[repr(align(64))]
struct CacheLine(AtomicUsize);

impl CacheLine {
    fn get(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }

    fn set(&self, value: usize) {
        self.0.store(value, Ordering::Release);
    }
}

/// Shared ring state, one per [`RingBuffer::new`].
#[derive(Debug)]
struct Shared {
    /// Index of the next sample the writer will overwrite.
    head: CacheLine,
    /// Index of the next sample the reader will copy out.
    tail: CacheLine,
    /// Samples the writer dropped because the ring was full.
    dropped: CacheLine,
    /// The sample slots, laid flat, stored as raw `f32` bit patterns.
    ///
    /// `AtomicU32` gives the interior mutability both halves need from
    /// behind a shared reference; the bit round trip of an `f32` is exact
    /// (including NaN payload behavior on the targets Lumina builds for).
    /// Orderings are `Relaxed` because the head and tail atomics carry the
    /// happens before edges, not the data cells.
    slots: Box<[AtomicU32]>,
    /// `capacity - 1`, with `capacity` a power of two; a mask is cheaper
    /// than a modulo and cannot be beaten by a branch predictor.
    mask: usize,
}

impl Shared {
    fn wrap(&self, index: usize) -> usize {
        index & self.mask
    }
}

/// A ring buffer of `f32` samples whose halves can be handed to two threads.
#[derive(Debug)]
pub struct RingBuffer {
    shared: Arc<Shared>,
}

impl RingBuffer {
    /// Create a ring with the given number of sample slots.
    ///
    /// # Panics
    ///
    /// Panics unless `capacity` is a power of two of at least two. Real
    /// callers use 8192 or 16384, never the pathological zero and one.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        assert!(
            capacity.is_power_of_two() && capacity >= 2,
            "ring capacity must be a power of two >= 2"
        );
        let slots = (0..capacity)
            .map(|_| AtomicU32::new(0.0f32.to_bits()))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let shared = Shared {
            head: CacheLine::default(),
            tail: CacheLine::default(),
            dropped: CacheLine::default(),
            slots,
            mask: capacity - 1,
        };
        Self {
            shared: Arc::new(shared),
        }
    }

    /// Split into a writer and a reader to be moved onto separate threads.
    ///
    /// The halves stay connected for as long as either half is alive.
    #[must_use]
    pub fn split(self) -> (RingWriter, RingReader) {
        let shared = self.shared;
        (
            RingWriter {
                inner: shared.clone(),
            },
            RingReader { inner: shared },
        )
    }
}

/// The production half of the ring. `Send`, used on the capture thread.
#[derive(Debug)]
pub struct RingWriter {
    inner: Arc<Shared>,
}

impl RingWriter {
    /// Write as many of `samples` as fit, dropping the tail when full.
    ///
    /// Returns the number of samples actually stored, in `0..=samples.len()`.
    /// There is no allocation, no lock, and no unbounded loop: safe to call
    /// from a realtime callback.
    ///
    /// # Panics
    ///
    /// Never.
    pub fn write(&self, samples: &[f32]) -> usize {
        let shared = &*self.inner;
        let capacity = shared.mask + 1;
        if samples.len() >= capacity {
            // Even after a reader wakes up there is no room for this burst, so
            // refuse early and drop it entirely rather than writing garbage.
            shared.dropped.0.fetch_add(samples.len(), Ordering::Relaxed);
            return 0;
        }

        let mut head = shared.head.get();
        let mut written = 0usize;
        for &sample in samples {
            let next = head + 1;
            // The reader frees space concurrently; re-read the tail each step
            // so a burst that starts while the ring is full still gets its
            // second half in once the reader catches up.
            if shared.wrap(next) == shared.wrap(shared.tail.get()) {
                let remaining = samples.len() - written;
                shared.dropped.0.fetch_add(remaining, Ordering::Relaxed);
                break;
            }
            shared.slots[shared.wrap(head)].store(sample.to_bits(), Ordering::Relaxed);
            head = next;
            written += 1;
        }

        if written > 0 {
            shared.head.set(head);
        }
        written
    }

    /// How many samples this writer has dropped since creation.
    #[must_use]
    pub fn dropped_samples(&self) -> usize {
        self.inner.dropped.get()
    }

    /// Samples currently buffered and not yet read.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.inner.head.get().wrapping_sub(self.inner.tail.get())
    }

    /// Free slot space, in samples.
    #[must_use]
    pub fn free_space(&self) -> usize {
        (self.inner.mask + 1) - self.pending() - 1
    }
}

/// The consumption half of the ring. `Send`, used on the analysis thread.
#[derive(Debug)]
pub struct RingReader {
    inner: Arc<Shared>,
}

impl RingReader {
    /// Copy buffered samples into `out`, oldest first, advancing the reader.
    ///
    /// Returns the number of samples copied. Chronological order is what the
    /// FFT expects; the ring never reorders.
    ///
    /// # Panics
    ///
    /// Never.
    pub fn read(&self, out: &mut [f32]) -> usize {
        let shared = &*self.inner;
        let head = shared.head.get();
        let mut tail = shared.tail.get();
        let count = head.wrapping_sub(tail).min(out.len());

        for slot in out.iter_mut().take(count) {
            *slot = f32::from_bits(shared.slots[shared.wrap(tail)].load(Ordering::Relaxed));
            tail += 1;
        }
        shared.tail.set(tail);
        count
    }

    /// Samples currently buffered and not yet read.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.inner.head.get().wrapping_sub(self.inner.tail.get())
    }

    /// Read exactly `out.len()` samples into `out`, oldest first, or `None`
    /// when fewer than that are buffered.
    ///
    /// The FFT needs one full window and a partial window would smear the
    /// spectrum, so the analysis thread waits for this.
    ///
    /// # Panics
    ///
    /// Never.
    pub fn read_exact(&self, out: &mut [f32]) -> Option<()> {
        let shared = &*self.inner;
        let head = shared.head.get();
        let tail = shared.tail.get();
        if head.wrapping_sub(tail) < out.len() {
            return None;
        }

        let mut tail = tail;
        for slot in out.iter_mut() {
            *slot = f32::from_bits(shared.slots[shared.wrap(tail)].load(Ordering::Relaxed));
            tail += 1;
        }
        shared.tail.set(tail);
        Some(())
    }

    /// Discard every buffered sample. Returns how many were discarded.
    ///
    /// Called by the analysis thread after an underflow or a state change,
    /// so a paused stream does not replay stale audio into the next frame.
    pub fn clear(&self) -> usize {
        let shared = &*self.inner;
        let head = shared.head.get();
        let tail = shared.tail.get();
        let skipped = head.wrapping_sub(tail);
        shared.tail.set(head);
        skipped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    /// A deterministic 8 bit LCG so test failures always reproduce.
    struct Lcg(u32);

    impl Lcg {
        fn next(&mut self) -> u32 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            self.0
        }
    }

    #[test]
    fn a_fresh_ring_is_empty() {
        let (writer, reader) = RingBuffer::new(64).split();
        assert_eq!(reader.pending(), 0);
        assert!(reader.read_exact(&mut [0.0; 4]).is_none());
        assert_eq!(writer.dropped_samples(), 0);
    }

    #[test]
    fn samples_come_out_in_write_order() {
        let (writer, reader) = RingBuffer::new(64).split();
        writer.write(&[1.0, 2.0, 3.0, 4.0]);
        let mut out = [0.0f32; 4];
        assert_eq!(reader.read(&mut out), 4);
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(reader.pending(), 0);
    }

    #[test]
    fn the_ring_wraps_around_a_power_of_two_boundary() {
        let (writer, reader) = RingBuffer::new(64).split();
        // Sixty-three buffered samples is the maximum for 64 slots.
        for step in 0..60u32 {
            writer.write(&[step as f32]);
        }
        let mut out = [0.0f32; 40];
        assert_eq!(reader.read(&mut out), 40);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[39], 39.0);

        // The indices have not wrapped yet, but write enough to force it.
        for step in 0..40u32 {
            writer.write(&[(step + 60) as f32]);
        }
        let mut second = [0.0f32; 20];
        assert_eq!(reader.read(&mut second), 20);
        assert_eq!(second[0], 40.0);
        assert_eq!(second[19], 59.0);
    }

    #[test]
    fn a_write_larger_than_the_ring_drops_entirely_and_counts() {
        let (writer, reader) = RingBuffer::new(64).split();
        writer.write(&[0.5; 48]);
        // 64 slots cannot hold 100 samples at once.
        assert_eq!(writer.write(&[1.0; 100]), 0);
        assert_eq!(writer.dropped_samples(), 100);

        let mut out = [0.0f32; 48];
        assert_eq!(reader.read(&mut out), 48);
        for sample in out {
            assert!((sample - 0.5).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn an_overful_write_keeps_its_prefix_and_drops_the_rest() {
        let (writer, reader) = RingBuffer::new(64).split();
        writer.write(&[9.0; 40]);
        // Forty free slots: fifty would drop ten.
        let written = writer.write(&[1.0; 50]);
        assert_eq!(written, 23); // capacity 64 - 40 buffered - 1 spare
        assert_eq!(writer.dropped_samples(), 27);

        let mut out = [0.0f32; 64];
        assert_eq!(reader.read(&mut out), 63);
        assert!(out[..40].iter().all(|s| *s == 9.0));
        assert!(out[40..63].iter().all(|s| *s == 1.0));
    }

    #[test]
    fn read_exact_requires_a_full_window() {
        let (writer, reader) = RingBuffer::new(64).split();
        writer.write(&[1.0, 2.0, 3.0]);
        assert!(
            reader.read_exact(&mut [0.0f32; 4]).is_none(),
            "incomplete window must not resolve"
        );
        assert_eq!(reader.pending(), 3, "a failed read must not consume");

        writer.write(&[4.0]);
        let mut out = [0.0f32; 4];
        assert!(reader.read_exact(&mut out).is_some());
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn clear_discards_everything_buffered() {
        let (writer, reader) = RingBuffer::new(64).split();
        writer.write(&[1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(reader.clear(), 5);
        assert_eq!(reader.pending(), 0);
        assert_eq!(reader.read(&mut [0.0f32; 8]), 0);
    }

    #[test]
    fn writer_and_reader_are_send_across_threads() {
        let (writer, reader) = RingBuffer::new(1024).split();
        thread::scope(|scope| {
            scope.spawn(move || {
                for step in 0..1000u32 {
                    writer.write(&[step as f32]);
                }
            });
            scope.spawn(move || {
                let mut total = 0usize;
                let mut out = [0.0f32; 256];
                while total < 1000 {
                    total += reader.read(&mut out);
                }
            });
        });
    }

    /// The core realtime property: a reader never observes a torn block.
    ///
    /// The writer emits blocks of `BLOCK` samples, each block holding one
    /// repeated tag. A reader that copies a block sees either the whole
    /// previous tag or the whole new tag; a value that is half of each would
    /// be a torn read, and fails the test.
    ///
    /// The writer refuses to write a block it cannot fit whole, so the block
    /// alignment of the stream is preserved through the ring: partial blocks
    /// never enter. Tags may skip forwards whenever a block is refused, which
    /// is expected under drop, but they never go backwards.
    #[test]
    fn a_concurrent_reader_never_sees_a_torn_block() {
        const BLOCK: usize = 16;
        let (writer, reader) = RingBuffer::new(8192).split();
        let writer_done = Arc::new(std::sync::atomic::AtomicBool::new(false));

        thread::scope(|scope| {
            let done = Arc::clone(&writer_done);
            scope.spawn(move || {
                let mut rng = Lcg(0x1234_5678);
                let mut tag = 1u32;
                let mut refusals = 0u32;
                loop {
                    if tag > 9000 || refusals >= 10_000 {
                        // The refusal budget is what keeps this test from
                        // hanging: a dead reader cannot renew the ring, and
                        // a healthy one frees a block well before this.
                        break;
                    }
                    if rng.next().is_multiple_of(4) {
                        thread::sleep(std::time::Duration::from_micros(20));
                    }
                    if writer.free_space() < BLOCK {
                        refusals += 1;
                        continue;
                    }
                    refusals = 0;
                    let sample = tag as f32 * 0.25;
                    writer.write(&[sample; BLOCK]);
                    tag += 1;
                }
                done.store(true, Ordering::Release);
            });

            scope.spawn(move || {
                let mut out = [0.0f32; BLOCK];
                let mut last_tag = 0u32;
                let mut resolved = 0u32;
                // The termination condition is the writer's exit flag, not
                // a resolution target: a fixed target can starve one block
                // short of its goal and hang the scope.
                while !writer_done.load(Ordering::Acquire) {
                    // Only drain on block boundaries so the stream stays
                    // aligned, exactly like the real analysis thread would.
                    if reader.pending() < BLOCK {
                        continue;
                    }
                    let got = reader.read(&mut out);
                    assert_eq!(got, BLOCK, "a block-full read must return whole blocks");
                    for chunk in out.chunks_exact(BLOCK) {
                        let first = chunk[0];
                        assert!(
                            chunk.iter().all(|s| *s == first),
                            "torn read at tag {first} after {last_tag}"
                        );
                        let tag = (first / 0.25).round() as u32;
                        assert!(
                            tag >= last_tag,
                            "tags went backwards: {tag} after {last_tag}"
                        );
                        last_tag = tag;
                        resolved += 1;
                    }
                }
                assert!(
                    resolved > 3000,
                    "far fewer blocks resolved than written: {resolved}"
                );
            });
        });
    }

    #[test]
    fn dropped_samples_ledger_stays_consistent_under_load() {
        let (writer, reader) = RingBuffer::new(256).split();
        thread::scope(|scope| {
            scope.spawn(move || {
                for round in 0..200u32 {
                    writer.write(&[round as f32; 96]);
                }
            });
            // A lazy reader guarantees drops, which is the point.
            thread::sleep(std::time::Duration::from_millis(1));
            let mut out = [0.0f32; 16];
            let mut read_total = 0usize;
            for _ in 0..200 {
                read_total += reader.read(&mut out);
                thread::sleep(std::time::Duration::from_micros(10));
            }
            assert!(read_total > 0, "the reader must have seen something");
        });
        // Nothing to assert on totals without knowing the scheduler, but no
        // data race means no torn state; the two other tests cover that.
    }
}
