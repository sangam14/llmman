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

/// Observability snapshot of the pool's current state.
#[derive(Debug, Clone)]
pub struct PoolStats {
    /// Number of VMs currently ready in the pool.
    pub warm: usize,
    /// Configured target capacity.
    pub target: usize,
    /// Total VMs ever dispensed from this pool instance.
    pub total_acquired: u64,
    /// How many of those came from the warm pool (vs. cold boot).
    pub pool_hits: u64,
}

pub struct VmPool {
    pool: Arc<Mutex<PoolInner>>,
    target_size: usize,
    bin_path: PathBuf,
    socket_dir: PathBuf,
}

struct PoolInner {
    vms: VecDeque<FirecrackerVm>,
    total_acquired: u64,
    pool_hits: u64,
}

impl VmPool {
    /// Create a new pre-warmed MicroVM pool.
    pub fn new(target_size: usize, bin_path: PathBuf, socket_dir: PathBuf) -> Self {
        Self {
            pool: Arc::new(Mutex::new(PoolInner {
                vms: VecDeque::new(),
                total_acquired: 0,
                pool_hits: 0,
            })),
            target_size,
            bin_path,
            socket_dir,
        }
    }

    /// Returns `true` if the Firecracker binary can be found.
    fn can_run(&self) -> bool {
        crate::find_on_path(self.bin_path.to_str().unwrap_or("")).is_some()
            || self.bin_path.exists()
            || self.bin_path == std::path::Path::new("firecracker")
    }

    /// Pre-warm the pool up to the target size.
    pub async fn warm(&self) -> Result<()> {
        if !self.can_run() {
            return Ok(());
        }

        // Spawn VMs outside the lock to avoid blocking acquire() callers.
        let mut new_vms = Vec::new();
        let current_len = self.pool.lock().await.vms.len();
        let needed = self.target_size.saturating_sub(current_len);

        for _ in 0..needed {
            let id = uuid_or_nanos();
            let socket_path = self.socket_dir.join(format!("fc-pool-{}.sock", id));
            if let Ok(vm) = FirecrackerVm::spawn(&self.bin_path, &socket_path) {
                new_vms.push(vm);
            }
        }

        // Push under lock.
        if !new_vms.is_empty() {
            let mut inner = self.pool.lock().await;
            for vm in new_vms {
                if inner.vms.len() < self.target_size {
                    inner.vms.push_back(vm);
                }
            }
        }

        Ok(())
    }

    /// Acquire a pre-warmed VM from the pool, asynchronously refilling the pool.
    ///
    /// Returns `None` if the pool is empty. VMs are health-checked before
    /// being returned — a VM whose process died is silently discarded.
    pub async fn acquire(&self) -> Result<Option<FirecrackerVm>> {
        let vm = {
            let mut inner = self.pool.lock().await;
            inner.total_acquired += 1;

            // Pop VMs until we find a healthy one or exhaust the pool.
            loop {
                match inner.vms.pop_front() {
                    Some(vm) if vm.is_healthy() => {
                        inner.pool_hits += 1;
                        break Some(vm);
                    }
                    Some(_stale) => {
                        // Dead VM — drop it and try the next one.
                        continue;
                    }
                    None => break None,
                }
            }
        }; // Lock released here.

        // Trigger async refill outside the lock.
        if self.can_run() {
            let pool_clone = Arc::clone(&self.pool);
            let target_size = self.target_size;
            let bin_path = self.bin_path.clone();
            let socket_dir = self.socket_dir.clone();

            tokio::spawn(async move {
                let mut new_vms = Vec::new();
                let current_len = pool_clone.lock().await.vms.len();
                let needed = target_size.saturating_sub(current_len);

                for _ in 0..needed {
                    let id = uuid_or_nanos();
                    let socket_path = socket_dir.join(format!("fc-pool-{}.sock", id));
                    if let Ok(vm) = FirecrackerVm::spawn(&bin_path, &socket_path) {
                        new_vms.push(vm);
                    }
                }

                if !new_vms.is_empty() {
                    let mut inner = pool_clone.lock().await;
                    for vm in new_vms {
                        if inner.vms.len() < target_size {
                            inner.vms.push_back(vm);
                        }
                    }
                }
            });
        }

        Ok(vm)
    }

    /// Return current number of pre-warmed VMs.
    pub async fn len(&self) -> usize {
        self.pool.lock().await.vms.len()
    }

    /// Check if the pool is empty.
    pub async fn is_empty(&self) -> bool {
        self.pool.lock().await.vms.is_empty()
    }

    /// Returns an observability snapshot of the pool's state.
    pub async fn stats(&self) -> PoolStats {
        let inner = self.pool.lock().await;
        PoolStats {
            warm: inner.vms.len(),
            target: self.target_size,
            total_acquired: inner.total_acquired,
            pool_hits: inner.pool_hits,
        }
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

    #[tokio::test]
    async fn test_pool_stats() {
        let temp_dir = tempfile::tempdir().unwrap();
        let pool = VmPool::new(
            3,
            PathBuf::from("/nonexistent/firecracker"),
            temp_dir.path().to_path_buf(),
        );

        let stats = pool.stats().await;
        assert_eq!(stats.warm, 0);
        assert_eq!(stats.target, 3);
        assert_eq!(stats.total_acquired, 0);
        assert_eq!(stats.pool_hits, 0);
    }
}
