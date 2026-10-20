use lumina_render::render::{FrameIntent, RenderLoop};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

struct CountingAllocator;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
fn an_idle_frame_loop_does_not_allocate() {
    let mut render_loop = RenderLoop::new(60, true);
    let idle = FrameIntent {
        dirty: false,
        animating: false,
        audio_active: false,
        visible: true,
    };

    assert!(!render_loop.should_render(idle));
    let before = ALLOCATIONS.load(Ordering::Relaxed);

    for _ in 0..1_000 {
        assert!(!render_loop.should_render(idle));
        render_loop.record_frame_time(Duration::from_millis(16));
    }

    let after = ALLOCATIONS.load(Ordering::Relaxed);
    assert_eq!(after, before, "the idle frame loop allocated");
}
