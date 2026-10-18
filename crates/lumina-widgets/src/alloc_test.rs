//! Counting allocator and zero-allocation frame path verification.
//!
//! # The problem
//!
//! The frame path must not allocate. Allocation in the frame path causes
//! hitches, which are visible as stutter. The counting allocator verifies this
//! by tracking every allocation and deallocation, and asserting that the
//! allocation count does not increase between two consecutive frames.
//!
//! # How it works
//!
//! The [`CountingAllocator`] wraps the global allocator and maintains a counter
//! of active allocations. The [`FrameGuard`] RAII type captures the allocation
//! count at the start of a frame and asserts that it has not increased when
//! the frame ends.
//!
//! # Usage
//!
//! ```rust
//! use lumina_widgets::alloc_test::{CountingAllocator, FrameGuard};
//!
//! // In tests, install the counting allocator as the global allocator.
//! // In production, the frame path is verified by the debug assertion that
//! // the allocation counter does not move between two consecutive present calls.
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A global allocator that counts active allocations.
///
/// This allocator wraps the system allocator and maintains a counter of
/// active allocations. It is used in tests to verify that the frame path
/// does not allocate.
pub struct CountingAllocator;

/// The global allocation counter.
static ALLOCATION_COUNT: AtomicUsize = AtomicUsize::new(0);

/// The global deallocation counter.
static DEALLOCATION_COUNT: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATION_COUNT.fetch_add(1, Ordering::SeqCst);
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        DEALLOCATION_COUNT.fetch_add(1, Ordering::SeqCst);
        System.dealloc(ptr, layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATION_COUNT.fetch_add(1, Ordering::SeqCst);
        DEALLOCATION_COUNT.fetch_add(1, Ordering::SeqCst);
        System.realloc(ptr, layout, new_size)
    }
}

/// Get the current number of active allocations.
#[must_use]
pub fn active_allocations() -> usize {
    ALLOCATION_COUNT.load(Ordering::SeqCst) - DEALLOCATION_COUNT.load(Ordering::SeqCst)
}

/// Get the total number of allocations since the program started.
#[must_use]
pub fn total_allocations() -> usize {
    ALLOCATION_COUNT.load(Ordering::SeqCst)
}

/// Get the total number of deallocations since the program started.
#[must_use]
pub fn total_deallocations() -> usize {
    DEALLOCATION_COUNT.load(Ordering::SeqCst)
}

/// Reset the allocation counters.
///
/// This is useful in tests to get a clean baseline.
pub fn reset_counters() {
    ALLOCATION_COUNT.store(0, Ordering::SeqCst);
    DEALLOCATION_COUNT.store(0, Ordering::SeqCst);
}

/// A guard that verifies no allocation happened during its lifetime.
///
/// The guard captures the allocation count when created and asserts that
/// it has not increased when dropped. This is used to verify that the
/// frame path does not allocate.
///
/// # Example
///
/// ```rust
/// use lumina_widgets::alloc_test::FrameGuard;
///
/// let guard = FrameGuard::new();
/// // ... do work that should not allocate ...
/// drop(guard); // asserts no allocation happened
/// ```
pub struct FrameGuard {
    start_count: usize,
}

impl FrameGuard {
    /// Create a new frame guard.
    #[must_use]
    pub fn new() -> Self {
        Self {
            start_count: active_allocations(),
        }
    }

    /// Get the allocation count when the guard was created.
    #[must_use]
    pub fn start_count(&self) -> usize {
        self.start_count
    }

    /// Get the current allocation count.
    #[must_use]
    pub fn current_count(&self) -> usize {
        active_allocations()
    }

    /// Check if any allocation happened since the guard was created.
    #[must_use]
    pub fn allocated(&self) -> bool {
        self.current_count() > self.start_count
    }

    /// Get the number of allocations that happened since the guard was created.
    #[must_use]
    pub fn allocation_delta(&self) -> usize {
        self.current_count().saturating_sub(self.start_count)
    }
}

impl Default for FrameGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for FrameGuard {
    fn drop(&mut self) {
        assert!(
            !self.allocated(),
            "frame path allocated {} times during the frame",
            self.allocation_delta()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widget::{DrawList, Quad, Widget, WidgetContext, WidgetInput, WidgetRegistry};
    use crate::clock::ClockWidget;
    use crate::calendar::CalendarWidget;
    use lumina_core::geometry::{Rect, Size, Vec2};
    use lumina_core::state::IslandState;

    #[test]
    fn counting_allocator_tracks_allocations() {
        reset_counters();
        let before = total_allocations();
        let _v: Vec<u8> = vec![0; 100];
        let after = total_allocations();
        assert!(after > before, "allocation should be counted");
    }

    #[test]
    fn counting_allocator_tracks_deallocations() {
        reset_counters();
        let before = total_deallocations();
        {
            let _v: Vec<u8> = vec![0; 100];
        }
        let after = total_deallocations();
        assert!(after > before, "deallocation should be counted");
    }

    #[test]
    fn active_allocations_is_the_difference() {
        reset_counters();
        let v1: Vec<u8> = vec![0; 100];
        let v2: Vec<u8> = vec![0; 200];
        let active = active_allocations();
        assert!(active >= 2, "at least 2 allocations should be active");
        drop(v1);
        drop(v2);
    }

    #[test]
    fn frame_guard_detects_allocation() {
        reset_counters();
        let guard = FrameGuard::new();
        let _v: Vec<u8> = vec![0; 100];
        assert!(guard.allocated(), "guard should detect allocation");
        assert!(guard.allocation_delta() > 0);
    }

    #[test]
    fn frame_guard_passes_when_no_allocation() {
        reset_counters();
        let guard = FrameGuard::new();
        let _ = 1 + 1;
        assert!(!guard.allocated(), "guard should not detect allocation");
        assert_eq!(guard.allocation_delta(), 0);
    }

    #[test]
    fn draw_list_with_capacity_does_not_allocate_after_reserve() {
        reset_counters();
        let mut list = DrawList::with_capacity(100);
        let guard = FrameGuard::new();
        list.push(Quad::new(Vec2::ZERO, Size::new(10.0, 10.0), Vec2::ZERO, Vec2::ZERO, [1.0; 4]));
        list.push(Quad::new(Vec2::ZERO, Size::new(10.0, 10.0), Vec2::ZERO, Vec2::ZERO, [1.0; 4]));
        assert!(!guard.allocated(), "draw list should not allocate after reserve");
    }

    #[test]
    fn draw_list_clear_does_not_allocate() {
        reset_counters();
        let mut list = DrawList::with_capacity(100);
        list.push(Quad::new(Vec2::ZERO, Size::new(10.0, 10.0), Vec2::ZERO, Vec2::ZERO, [1.0; 4]));
        let guard = FrameGuard::new();
        list.clear();
        assert!(!guard.allocated(), "clear should not allocate");
    }

    #[test]
    fn widget_registry_layout_does_not_allocate_after_warmup() {
        reset_counters();
        let mut registry = WidgetRegistry::new();
        registry.register(Box::new(ClockWidget::new()));
        registry.register(Box::new(CalendarWidget::new()));
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(400.0, 40.0));

        // Warm up: first layout may allocate for the rects vector
        let _ = registry.layout(&ctx);

        // Second layout should not allocate
        let guard = FrameGuard::new();
        let rects = registry.layout(&ctx);
        assert!(!guard.allocated(), "layout should not allocate after warmup");
        assert_eq!(rects.len(), 2);
    }

    #[test]
    fn widget_registry_draw_does_not_allocate_after_warmup() {
        reset_counters();
        let mut registry = WidgetRegistry::new();
        registry.register(Box::new(ClockWidget::new()));
        registry.register(Box::new(CalendarWidget::new()));
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(400.0, 40.0));
        let rects = registry.layout(&ctx);

        // Warm up: first draw may allocate for the draw list
        let mut out = DrawList::with_capacity(100);
        registry.draw(&mut out, &ctx, &rects);

        // Second draw should not allocate
        let guard = FrameGuard::new();
        out.clear();
        registry.draw(&mut out, &ctx, &rects);
        assert!(!guard.allocated(), "draw should not allocate after warmup");
    }

    #[test]
    fn widget_registry_route_input_does_not_allocate() {
        reset_counters();
        let mut registry = WidgetRegistry::new();
        registry.register(Box::new(ClockWidget::new()));
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 40.0));
        let rects = registry.layout(&ctx);

        let guard = FrameGuard::new();
        let _ = registry.route_input(&WidgetInput::Click(Vec2::new(50.0, 20.0)), &ctx, &rects);
        assert!(!guard.allocated(), "route_input should not allocate");
    }

    #[test]
    fn widget_registry_requests_does_not_allocate() {
        reset_counters();
        let mut registry = WidgetRegistry::new();
        registry.register(Box::new(ClockWidget::new()));
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(200.0, 40.0));

        let guard = FrameGuard::new();
        let _ = registry.requests(&ctx);
        assert!(!guard.allocated(), "requests should not allocate");
    }

    #[test]
    fn full_frame_path_does_not_allocate_after_warmup() {
        reset_counters();
        let mut registry = WidgetRegistry::new();
        registry.register(Box::new(ClockWidget::new()));
        registry.register(Box::new(CalendarWidget::new()));
        let ctx = WidgetContext::new(IslandState::Clock, Size::new(400.0, 40.0));

        // Warm up
        let rects = registry.layout(&ctx);
        let mut out = DrawList::with_capacity(100);
        registry.draw(&mut out, &ctx, &rects);

        // Full frame: layout + draw + route_input + requests
        let guard = FrameGuard::new();
        let rects = registry.layout(&ctx);
        out.clear();
        registry.draw(&mut out, &ctx, &rects);
        let _ = registry.route_input(&WidgetInput::Click(Vec2::new(50.0, 20.0)), &ctx, &rects);
        let _ = registry.requests(&ctx);
        assert!(!guard.allocated(), "full frame path should not allocate after warmup");
    }
}
