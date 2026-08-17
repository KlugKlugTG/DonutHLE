use std::sync::atomic::{AtomicU32, Ordering};

static FRAME_COUNT: AtomicU32 = AtomicU32::new(0);

pub fn next_frame() -> u32 {
    FRAME_COUNT.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
}
