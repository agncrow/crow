//! `web.queue` module — Background job processing.
//!
//! ```text
//! use web.queue
//!
//! q = queue[4]  --- 4 worker threads ---
//! push[q, fn[]
//!   log['background work']
//! ]
//! ```

use std::sync::Arc;

/// A simple background job queue using tokio tasks.
#[derive(Clone)]
pub struct CrowQueue {
    /// Max concurrent jobs.
    max_workers: usize,
    /// Semaphore to limit concurrency.
    semaphore: Arc<tokio::sync::Semaphore>,
}

impl CrowQueue {
    /// Create a queue with `max_workers` concurrent job slots.
    pub fn new(max_workers: usize) -> Self {
        Self {
            max_workers,
            semaphore: Arc::new(tokio::sync::Semaphore::new(max_workers)),
        }
    }

    /// Push a job onto the queue. Runs in the background.
    /// Returns immediately. The job will execute when a worker slot is available.
    pub fn push<F>(&self, job: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let sem = self.semaphore.clone();
        tokio::spawn(async move {
            // a closed queue runs nothing (it is never closed)
            let Ok(_permit) = sem.acquire().await else { return };
            tokio::task::spawn_blocking(job).await.ok();
        });
    }

    /// Push an async job onto the queue.
    pub fn push_async<F, Fut>(&self, job: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let sem = self.semaphore.clone();
        tokio::spawn(async move {
            let Ok(_permit) = sem.acquire().await else { return };
            job().await;
        });
    }

    /// Get the number of available worker slots.
    pub fn available(&self) -> usize {
        self.semaphore.available_permits()
    }

    /// Get the max workers count.
    pub fn workers(&self) -> usize {
        self.max_workers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_queue_creation() {
        let q = CrowQueue::new(4);
        assert_eq!(q.workers(), 4);
        assert_eq!(q.available(), 4);
    }

    #[tokio::test]
    async fn test_queue_push_executes() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();

        let q = CrowQueue::new(2);
        q.push(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });

        // Give the task time to run
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_queue_push_async_executes() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();

        let q = CrowQueue::new(2);
        q.push_async(move || async move {
            c.fetch_add(1, Ordering::SeqCst);
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_queue_multiple_jobs() {
        let counter = Arc::new(AtomicUsize::new(0));
        let q = CrowQueue::new(4);

        for _ in 0..10 {
            let c = counter.clone();
            q.push(move || {
                c.fetch_add(1, Ordering::SeqCst);
            });
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert_eq!(counter.load(Ordering::SeqCst), 10);
    }
}

