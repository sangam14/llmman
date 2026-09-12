//! Pre-warmed Firecracker MicroVM Pool.
//!
//! Maintains a queue of pre-booted Firecracker instances ready for immediate work,
//! reducing container start latency to under 100ms.

use anyhow::Result;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::runtime::firecracker::FirecrackerVm;

pub struct VmPool {
    pool: Arc<Mutex<VecDeque<FirecrackerVm>>>,
    target_size: usize,
    bin_path: PathBuf,
    socket_dir: PathBuf,
}

impl VmPool {
    /// Create a new pre-warmed MicroVM pool.
    pub fn new(target_size: usize, bin_path: PathBuf, socket_dir: PathBuf) -> Self {
        Self {
            pool: Arc::new(Mutex::new(VecDeque::new())),
            target_size,
            bin_path,
            socket_dir,
        }
    }

    /// Pre-warm the pool up to the target size.
    pub async fn warm(&self) -> Result<()> {
        let mut pool_lock = self.pool.lock().await;
        let mut attempts = 0;
        while pool_lock.len() < self.target_size && attempts < self.target_size {
            attempts += 1;
            let id = uuid_or_nanos();
            let socket_path = self.socket_dir.join(format!("fc-pool-{}.sock", id));

            let can_run = crate::find_on_path(self.bin_path.to_str().unwrap_or("")).is_some()
                || self.bin_path.exists()
                || self.bin_path == std::path::Path::new("firecracker");
            if can_run {
                if let Ok(vm) = FirecrackerVm::spawn(&self.bin_path, &socket_path) {
                    pool_lock.push_back(vm);
                }
            }
        }
        Ok(())
    }

    /// Acquire a pre-warmed VM from the pool, asynchronously refilling the pool.
    pub async fn acquire(&self) -> Result<Option<FirecrackerVm>> {
        let mut pool_lock = self.pool.lock().await;
        let vm = pool_lock.pop_front();

        // Trigger async refill if below target
        if pool_lock.len() < self.target_size {
            let pool_clone = Arc::clone(&self.pool);
            let target_size = self.target_size;
            let bin_path = self.bin_path.clone();
            let socket_dir = self.socket_dir.clone();

            tokio::spawn(async move {
                let mut lock = pool_clone.lock().await;
                let mut attempts = 0;
                while lock.len() < target_size && attempts < target_size {
                    attempts += 1;
                    let can_run = crate::find_on_path(bin_path.to_str().unwrap_or("")).is_some()
                        || bin_path.exists()
                        || bin_path == std::path::Path::new("firecracker");
                    if can_run {
                        let id = uuid_or_nanos();
                        let socket_path = socket_dir.join(format!("fc-pool-{}.sock", id));
                        if let Ok(vm) = FirecrackerVm::spawn(&bin_path, &socket_path) {
                            lock.push_back(vm);
                        }
                    }
                }
            });
        }

        Ok(vm)
    }

    /// Return current number of pre-warmed VMs.
    pub async fn len(&self) -> usize {
        self.pool.lock().await.len()
    }

    /// Check if the pool is empty.
    pub async fn is_empty(&self) -> bool {
        self.pool.lock().await.is_empty()
    }
}

fn uuid_or_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_vm_pool_initialization() {
        let temp_dir = tempfile::tempdir().unwrap();
        let pool = VmPool::new(
            3,
            PathBuf::from("/nonexistent/firecracker"),
            temp_dir.path().to_path_buf(),
        );

        assert_eq!(pool.len().await, 0);
        assert!(pool.is_empty().await);
    }

    #[tokio::test]
    async fn test_vm_pool_acquire_empty() {
        let temp_dir = tempfile::tempdir().unwrap();
        let pool = VmPool::new(
            3,
            PathBuf::from("/nonexistent/firecracker"),
            temp_dir.path().to_path_buf(),
        );

        let acquired = pool.acquire().await.unwrap();
        assert!(acquired.is_none());
    }

    #[tokio::test]
    async fn test_vm_pool_warm_nonexistent_binary() {
        let temp_dir = tempfile::tempdir().unwrap();
        let pool = VmPool::new(
            3,
            PathBuf::from("/nonexistent/firecracker"),
            temp_dir.path().to_path_buf(),
        );

        pool.warm().await.unwrap();
        assert_eq!(pool.len().await, 0);
    }
}
