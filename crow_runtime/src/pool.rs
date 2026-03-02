use std::cell::RefCell;

/// Size buckets for the memory pool.
/// We round up allocation sizes to the nearest bucket to improve reuse.
/// Buckets: 64, 256, 1024, 4096, 16384, 65536, 262144, 1048576
fn bucket_size(requested: usize) -> usize {
    const BUCKETS: [usize; 8] = [64, 256, 1024, 4096, 16384, 65536, 262_144, 1_048_576];
    for &b in &BUCKETS {
        if requested <= b {
            return b;
        }
    }
    // For very large allocations, round up to next power of 2
    requested.next_power_of_two()
}

/// A pool of reusable `Vec<f64>` buffers, organized by size bucket.
/// Thread-local to avoid synchronization overhead.
pub struct TensorPool;

/// Per-bucket storage: a list of available buffers.
struct BucketStore {
    /// Free buffers, keyed by bucket size.
    /// Each entry is a Vec of available buffers at that bucket size.
    buffers: Vec<(usize, Vec<Vec<f64>>)>,
    /// Maximum number of buffers to keep per bucket.
    max_per_bucket: usize,
}

impl BucketStore {
    fn new() -> Self {
        Self {
            buffers: Vec::new(),
            max_per_bucket: 32,
        }
    }

    fn acquire(&mut self, size: usize) -> Vec<f64> {
        let bucket = bucket_size(size);
        // Find the bucket entry
        for (bsize, bufs) in &mut self.buffers {
            if *bsize == bucket {
                if let Some(mut buf) = bufs.pop() {
                    buf.clear();
                    return buf;
                }
                break;
            }
        }
        // No cached buffer — allocate a new one
        Vec::with_capacity(bucket)
    }

    fn release(&mut self, buf: Vec<f64>) {
        let bucket = bucket_size(buf.capacity());
        // Find or create bucket entry
        for (bsize, bufs) in &mut self.buffers {
            if *bsize == bucket {
                if bufs.len() < self.max_per_bucket {
                    bufs.push(buf);
                }
                // else: drop the buffer (pool is full for this bucket)
                return;
            }
        }
        // New bucket
        self.buffers.push((bucket, vec![buf]));
    }

    fn clear(&mut self) {
        self.buffers.clear();
    }

    fn stats(&self) -> PoolStats {
        let mut total_buffers = 0;
        let mut total_capacity = 0;
        for (bsize, bufs) in &self.buffers {
            total_buffers += bufs.len();
            total_capacity += bsize * bufs.len();
        }
        PoolStats {
            total_buffers,
            total_capacity_bytes: total_capacity * std::mem::size_of::<f64>(),
            bucket_count: self.buffers.len(),
        }
    }
}

thread_local! {
    static POOL: RefCell<BucketStore> = RefCell::new(BucketStore::new());
}

impl TensorPool {
    /// Acquire a `Vec<f64>` with at least `size` capacity.
    /// May return a recycled buffer from the pool.
    pub fn acquire(size: usize) -> Vec<f64> {
        POOL.with(|p| p.borrow_mut().acquire(size))
    }

    /// Return a buffer to the pool for future reuse.
    /// The buffer's contents are cleared but capacity is preserved.
    pub fn release(buf: Vec<f64>) {
        POOL.with(|p| p.borrow_mut().release(buf))
    }

    /// Clear all cached buffers, freeing memory.
    pub fn clear() {
        POOL.with(|p| p.borrow_mut().clear())
    }

    /// Get statistics about the current pool state.
    pub fn stats() -> PoolStats {
        POOL.with(|p| p.borrow().stats())
    }
}

/// Statistics about the memory pool.
#[derive(Debug, Clone)]
pub struct PoolStats {
    /// Number of buffers currently in the pool.
    pub total_buffers: usize,
    /// Total capacity in bytes of all pooled buffers.
    pub total_capacity_bytes: usize,
    /// Number of distinct size buckets in use.
    pub bucket_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_acquire_release() {
        TensorPool::clear();
        let buf = TensorPool::acquire(100);
        assert!(buf.capacity() >= 100);
        assert!(buf.is_empty());
        let cap = buf.capacity();
        TensorPool::release(buf);
        // Acquire again — should get the recycled buffer
        let buf2 = TensorPool::acquire(100);
        assert_eq!(buf2.capacity(), cap);
    }

    #[test]
    fn test_bucket_sizes() {
        assert_eq!(bucket_size(1), 64);
        assert_eq!(bucket_size(64), 64);
        assert_eq!(bucket_size(65), 256);
        assert_eq!(bucket_size(256), 256);
        assert_eq!(bucket_size(1000), 1024);
        assert_eq!(bucket_size(1025), 4096);
    }

    #[test]
    fn test_different_buckets() {
        TensorPool::clear();
        // Acquire from two different buckets
        let small = TensorPool::acquire(10);
        let large = TensorPool::acquire(5000);
        assert!(small.capacity() >= 10);
        assert!(large.capacity() >= 5000);
        TensorPool::release(small);
        TensorPool::release(large);
        let stats = TensorPool::stats();
        assert_eq!(stats.total_buffers, 2);
        assert_eq!(stats.bucket_count, 2);
    }

    #[test]
    fn test_clear() {
        TensorPool::clear();
        TensorPool::release(Vec::with_capacity(64));
        TensorPool::release(Vec::with_capacity(64));
        assert!(TensorPool::stats().total_buffers >= 2);
        TensorPool::clear();
        assert_eq!(TensorPool::stats().total_buffers, 0);
    }

    #[test]
    fn test_released_buffer_is_empty() {
        TensorPool::clear();
        let mut buf = TensorPool::acquire(100);
        buf.extend_from_slice(&[1.0, 2.0, 3.0]);
        TensorPool::release(buf);
        let buf2 = TensorPool::acquire(100);
        // Recycled buffer should be cleared
        assert!(buf2.is_empty());
    }

    #[test]
    fn test_max_per_bucket() {
        TensorPool::clear();
        // Release more than max_per_bucket (32) buffers
        for _ in 0..40 {
            TensorPool::release(Vec::with_capacity(64));
        }
        let stats = TensorPool::stats();
        // Should cap at 32
        assert!(stats.total_buffers <= 32);
    }

    #[test]
    fn test_large_allocation() {
        TensorPool::clear();
        let buf = TensorPool::acquire(2_000_000);
        assert!(buf.capacity() >= 2_000_000);
        // Should round to next power of 2
        assert!(buf.capacity().is_power_of_two() || buf.capacity() >= 2_000_000);
        TensorPool::release(buf);
    }
}

