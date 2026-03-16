use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::ptr::NonNull;

/// Heap-allocated buffer with guaranteed alignment to a specific boundary.
///
/// Required for unbuffered (direct) I/O on both Windows (`FILE_FLAG_NO_BUFFERING`)
/// and Linux (`O_DIRECT`), which mandate sector-aligned buffer addresses, offsets,
/// and transfer sizes.
pub struct AlignedBuffer {
    ptr: NonNull<u8>,
    layout: Layout,
    len: usize,
}

impl AlignedBuffer {
    /// Allocate a zero-initialised buffer of `size` bytes aligned to `alignment`.
    ///
    /// # Panics
    /// Panics if `alignment` is not a power of two, `size` is 0, or allocation fails.
    pub fn new(size: usize, alignment: usize) -> Self {
        assert!(size > 0, "AlignedBuffer: size must be > 0");
        assert!(
            alignment.is_power_of_two(),
            "AlignedBuffer: alignment must be a power of two"
        );
        let layout =
            Layout::from_size_align(size, alignment).expect("AlignedBuffer: invalid layout");
        // SAFETY: layout is valid — size > 0 and alignment is a power of two.
        let raw = unsafe { alloc_zeroed(layout) };
        let ptr = NonNull::new(raw).expect("AlignedBuffer: allocation failed");
        Self {
            ptr,
            layout,
            len: size,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn alignment(&self) -> usize {
        self.layout.align()
    }

    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: ptr is valid, non-null, correctly aligned; len bytes were allocated.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: ptr is valid, non-null, correctly aligned; len bytes were allocated.
        // &mut self guarantees exclusive access — no aliasing possible.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        // SAFETY: ptr was allocated with this exact layout and has not been freed.
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) }
    }
}

// SAFETY: AlignedBuffer exclusively owns its allocation; no aliasing possible.
unsafe impl Send for AlignedBuffer {}
unsafe impl Sync for AlignedBuffer {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alignment_is_respected() {
        for &align in &[512usize, 4096, 8192] {
            let buf = AlignedBuffer::new(align * 4, align);
            let ptr_val = buf.as_slice().as_ptr() as usize;
            assert_eq!(ptr_val % align, 0, "alignment {align} not satisfied");
        }
    }

    #[test]
    fn zero_initialised() {
        let buf = AlignedBuffer::new(512, 512);
        assert!(buf.as_slice().iter().all(|&b| b == 0));
    }

    #[test]
    fn write_and_read_back() {
        let mut buf = AlignedBuffer::new(512, 512);
        buf.as_mut_slice()[0] = 0xDE;
        buf.as_mut_slice()[511] = 0xAD;
        assert_eq!(buf.as_slice()[0], 0xDE);
        assert_eq!(buf.as_slice()[511], 0xAD);
    }

    #[test]
    fn len_matches_requested() {
        let buf = AlignedBuffer::new(1024, 512);
        assert_eq!(buf.len(), 1024);
        assert!(!buf.is_empty());
    }
}
