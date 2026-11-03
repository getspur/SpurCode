//! Test-only failure injection for real native tensor construction.
use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

static FAIL_AT: AtomicU8 = AtomicU8::new(0);
static RELEASES: AtomicUsize = AtomicUsize::new(0);
static ALLOCATOR_RELEASES: AtomicUsize = AtomicUsize::new(0);
static COMPILER_BUFFER_FREES: AtomicUsize = AtomicUsize::new(0);
static ALLOCATOR_FREES: AtomicUsize = AtomicUsize::new(0);

/// Observe calls to the real allocator free callback in focused tests.
pub fn reset_allocator_free_count() {
	ALLOCATOR_FREES.store(0, Ordering::SeqCst);
}

pub fn allocator_free_count() -> usize {
	ALLOCATOR_FREES.load(Ordering::SeqCst)
}

pub(crate) fn record_allocator_free() {
	ALLOCATOR_FREES.fetch_add(1, Ordering::SeqCst);
}

pub fn fail_next_compiler_after_output() {
	COMPILER_BUFFER_FREES.store(0, Ordering::SeqCst);
	FAIL_AT.store(4, Ordering::SeqCst);
}

pub fn compiler_buffer_free_count() -> usize {
	COMPILER_BUFFER_FREES.load(Ordering::SeqCst)
}

#[allow(dead_code)]
pub(crate) fn record_compiler_buffer_free() {
	COMPILER_BUFFER_FREES.fetch_add(1, Ordering::SeqCst);
}

pub fn reset_allocator_release_count() {
	ALLOCATOR_RELEASES.store(0, Ordering::SeqCst);
}

pub fn allocator_release_count() -> usize {
	ALLOCATOR_RELEASES.load(Ordering::SeqCst)
}

pub(crate) fn record_allocator_release() {
	ALLOCATOR_RELEASES.fetch_add(1, Ordering::SeqCst);
}

/// Select the next failure site: 1 for numeric mutable-data, 2 for string fill.
pub fn fail_next_tensor_step(site: u8) {
	RELEASES.store(0, Ordering::SeqCst);
	FAIL_AT.store(site, Ordering::SeqCst);
}

/// Intercept editor node creation before any native attribute transfer.
pub fn fail_next_editor_node() {
	FAIL_AT.store(3, Ordering::SeqCst);
}

pub fn clear_failpoint() {
	FAIL_AT.store(0, Ordering::SeqCst);
}

pub fn native_release_count() -> usize {
	RELEASES.load(Ordering::SeqCst)
}

pub(crate) fn fail_if_requested(site: u8) -> crate::Result<()> {
	if FAIL_AT.compare_exchange(site, 0, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
		return Err(crate::Error::new("injected tensor construction failure"));
	}
	Ok(())
}

#[allow(dead_code)]
pub(crate) fn record_native_release() {
	RELEASES.fetch_add(1, Ordering::SeqCst);
}
