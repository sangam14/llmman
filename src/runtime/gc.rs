//! Automated Garbage Collection & Crash Convergence for MicroVM Resources.
//!
//! Inspired by Cocoon's lock-safe GC sweeper, this module cleans up:
//! 1. Stale VM records whose hypervisor PIDs are dead (preserving valid hibernated snapshots).
//! 2. Orphaned CNI TAP network devices (`vmtap-*`).
//! 3. Leaked Unix Domain Sockets from crashed or killed instances.
//! 4. Snapshot LRU retention quotas (count, age, and total disk size limits).

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::runtime::{cni, lifecycle, process};

/// Observability report detailing resources reclaimed during a GC sweep.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcReport {
    pub purged_records: usize,
    pub cleaned_taps: usize,
    pub removed_sockets: usize,
    pub reclaimed_snapshots: usize,
    pub reclaimed_snapshot_bytes: u64,
}

impl GcReport {
    pub fn is_clean(&self) -> bool {
        self.purged_records == 0
            && self.cleaned_taps == 0
            && self.removed_sockets == 0
            && self.reclaimed_snapshots == 0
    }
}

/// Retention configuration for MicroVM memory and state snapshots.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotRetentionPolicy {
    /// Maximum number of snapshots to retain (LRU eviction of oldest).
    pub max_snapshots: Option<usize>,
    /// Maximum age of snapshots in seconds before eviction.
    pub max_age_secs: Option<u64>,
    /// Maximum total disk usage in bytes for snapshots directory.
    pub max_total_bytes: Option<u64>,
}

struct SnapshotUnit {
    path: PathBuf,
    size_bytes: u64,
    mtime: SystemTime,
    is_dir: bool,
    vm_id: Option<String>,
}

fn measure_dir(path: &Path) -> (u64, SystemTime) {
    let mut total_size = 0u64;
    let mut latest_mtime = SystemTime::UNIX_EPOCH;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                total_size += meta.len();
                if let Ok(mtime) = meta.modified() {
                    if mtime > latest_mtime {
                        latest_mtime = mtime;
                    }
                }
            }
        }
    }
    (total_size, latest_mtime)
}

/// Sweeps and reconciles all orphaned MicroVM resources using default retention (no LRU evictions).
pub fn sweep_orphaned_resources() -> Result<GcReport> {
    sweep_resources_with_policy(&SnapshotRetentionPolicy::default())
}

/// Sweeps and reconciles all orphaned MicroVM resources and enforces the specified snapshot retention policy.
pub fn sweep_resources_with_policy(policy: &SnapshotRetentionPolicy) -> Result<GcReport> {
    let mut report = GcReport::default();
    let dir = lifecycle::vms_dir();
    let snap_dir = lifecycle::snapshots_dir();

    let mut active_taps = std::collections::HashSet::new();

    // 1. Reconcile VM JSON records
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(record) = serde_json::from_str::<lifecycle::MicrovmRecord>(&content) {
                        let is_hibernated =
                            record.status == "HIBERNATED" || record.status == "hibernated";

                        if is_hibernated {
                            let vm_snap_dir = snap_dir.join(&record.id);
                            if vm_snap_dir.exists() {
                                // Preserved scale-to-zero instance
                                continue;
                            } else {
                                // Missing snapshot payload: purge stale record
                                let _ = std::fs::remove_file(&path);
                                report.purged_records += 1;
                            }
                        } else if process::is_alive(record.pid) {
                            active_taps.insert(record.tap.clone());
                        } else {
                            // PID is dead — tear down its remaining TAP and socket
                            if !record.tap.is_empty() && cni::tap_exists(&record.tap) {
                                let _ = cni::teardown_cni_network(
                                    &record.id,
                                    Path::new("/tmp"),
                                    "eth0",
                                    &record.tap,
                                );
                                report.cleaned_taps += 1;
                            }

                            let sock_path = Path::new(&record.socket_path);
                            if sock_path.exists() {
                                let _ = std::fs::remove_file(sock_path);
                                report.removed_sockets += 1;
                            }

                            let _ = std::fs::remove_file(&path);
                            report.purged_records += 1;
                        }
                    }
                }
            }
        }
    }

    // 2. Reconcile any orphaned host TAP devices not owned by a living VM
    for tap in cni::list_llmman_taps() {
        if !active_taps.contains(&tap)
            && cni::teardown_cni_network("orphaned", Path::new("/tmp"), "eth0", &tap).is_ok()
        {
            report.cleaned_taps += 1;
        }
    }

    // 3. Enforce Snapshot LRU Retention Quotas
    if policy.max_snapshots.is_some()
        || policy.max_age_secs.is_some()
        || policy.max_total_bytes.is_some()
    {
        sweep_snapshots_lru_in_dir(&snap_dir, policy, &mut report)?;
    }

    Ok(report)
}

fn sweep_snapshots_lru_in_dir(
    snap_dir: &Path,
    policy: &SnapshotRetentionPolicy,
    report: &mut GcReport,
) -> Result<()> {
    if !snap_dir.exists() {
        return Ok(());
    }

    let mut units = Vec::new();
    if let Ok(entries) = std::fs::read_dir(snap_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let (size, mtime) = measure_dir(&path);
                let vm_id = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string());
                units.push(SnapshotUnit {
                    path,
                    size_bytes: size,
                    mtime,
                    is_dir: true,
                    vm_id,
                });
            } else if let Ok(meta) = path.metadata() {
                let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                units.push(SnapshotUnit {
                    path,
                    size_bytes: meta.len(),
                    mtime,
                    is_dir: false,
                    vm_id: None,
                });
            }
        }
    }

    // Sort newest first
    units.sort_by_key(|a| std::cmp::Reverse(a.mtime));

    let now = SystemTime::now();
    let mut kept_bytes = 0u64;
    let mut evicted = Vec::new();

    for (idx, unit) in units.into_iter().enumerate() {
        let mut should_evict = false;

        // Check age limit
        if let Some(max_age) = policy.max_age_secs {
            if let Ok(age) = now.duration_since(unit.mtime) {
                if age.as_secs() > max_age {
                    should_evict = true;
                }
            }
        }

        // Check max snapshot count limit
        if let Some(max_count) = policy.max_snapshots {
            if idx >= max_count {
                should_evict = true;
            }
        }

        // Check max total bytes limit
        if let Some(max_bytes) = policy.max_total_bytes {
            if kept_bytes + unit.size_bytes > max_bytes {
                should_evict = true;
            }
        }

        if should_evict {
            evicted.push(unit);
        } else {
            kept_bytes += unit.size_bytes;
        }
    }

    // Delete evicted snapshots and remove corresponding hibernated VM records
    for unit in evicted {
        if unit.is_dir {
            let _ = std::fs::remove_dir_all(&unit.path);
        } else {
            let _ = std::fs::remove_file(&unit.path);
        }

        if let Some(vm_id) = &unit.vm_id {
            let vm_record_file = lifecycle::vms_dir().join(format!("{vm_id}.json"));
            if vm_record_file.exists() {
                let _ = std::fs::remove_file(vm_record_file);
                report.purged_records += 1;
            }
        }

        report.reclaimed_snapshots += 1;
        report.reclaimed_snapshot_bytes += unit.size_bytes;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gc_sweep_cleans_dead_records() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sock_path = temp_dir.path().join("dead.sock");
        std::fs::write(&sock_path, b"mock-socket").unwrap();

        let dead_record = lifecycle::MicrovmRecord {
            id: "vm-dead-test".to_string(),
            pid: 99999999, // guaranteed dead pid
            status: "running".to_string(),
            kernel: "vmlinux".to_string(),
            vcpus: 2,
            memory_mb: 2048,
            ip: "172.16.0.2".to_string(),
            tap: "".to_string(),
            model: "test".to_string(),
            started_at: "2026-09-13T00:00:00Z".to_string(),
            socket_path: sock_path.display().to_string(),
            snapshot_path: None,
            hibernated_at: None,
        };

        lifecycle::save_vm_record(&dead_record).unwrap();

        let report = sweep_orphaned_resources().unwrap();
        assert!(report.purged_records >= 1);
        assert!(!sock_path.exists());
    }

    #[test]
    fn test_gc_preserves_hibernated_record_with_snapshot() {
        let vm_id = format!("vm-hib-preserve-{}", std::process::id());
        let snap_dir = lifecycle::snapshots_dir().join(&vm_id);
        std::fs::create_dir_all(&snap_dir).unwrap();
        std::fs::write(snap_dir.join("vmstate"), b"state").unwrap();

        let record = lifecycle::MicrovmRecord {
            id: vm_id.clone(),
            pid: 99999998, // dead PID because hypervisor stopped
            status: "HIBERNATED".to_string(),
            kernel: "vmlinux".to_string(),
            vcpus: 2,
            memory_mb: 2048,
            ip: "172.16.0.2".to_string(),
            tap: "".to_string(),
            model: "test-model".to_string(),
            started_at: "2026-09-13T00:00:00Z".to_string(),
            socket_path: "".to_string(),
            snapshot_path: Some(snap_dir.display().to_string()),
            hibernated_at: Some("2026-09-13T01:00:00Z".to_string()),
        };

        lifecycle::save_vm_record(&record).unwrap();

        // Run standard GC
        let _report = sweep_orphaned_resources().unwrap();

        // Must NOT be purged because snapshot exists!
        let record_path = lifecycle::vms_dir().join(format!("{vm_id}.json"));
        assert!(record_path.exists(), "Hibernated record must be preserved");

        // Clean up test files
        let _ = std::fs::remove_dir_all(&snap_dir);
        let _ = std::fs::remove_file(&record_path);
    }

    #[test]
    fn test_snapshot_retention_policy_lru_eviction() {
        let temp_snap = tempfile::tempdir().unwrap();
        let snap_dir = temp_snap.path();

        // Create 3 snapshot directories with artificial delays to guarantee distinct mtimes
        for i in 1..=3 {
            let s_dir = snap_dir.join(format!("vm-snap-{i}"));
            std::fs::create_dir_all(&s_dir).unwrap();
            std::fs::write(s_dir.join("vmstate"), vec![0u8; 1000]).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(15));
        }

        let policy = SnapshotRetentionPolicy {
            max_snapshots: Some(1), // Retain only the most recent 1
            max_age_secs: None,
            max_total_bytes: None,
        };

        let mut report = GcReport::default();
        sweep_snapshots_lru_in_dir(snap_dir, &policy, &mut report).unwrap();

        assert_eq!(report.reclaimed_snapshots, 2);
        assert_eq!(report.reclaimed_snapshot_bytes, 2000);
        assert!(!snap_dir.join("vm-snap-1").exists());
        assert!(!snap_dir.join("vm-snap-2").exists());
        assert!(snap_dir.join("vm-snap-3").exists());
    }

    #[test]
    fn test_snapshot_retention_policy_max_bytes() {
        let temp_snap = tempfile::tempdir().unwrap();
        let snap_dir = temp_snap.path();

        // Create 2 snapshots of 500 bytes each
        for i in 1..=2 {
            let s_dir = snap_dir.join(format!("vm-byte-{i}"));
            std::fs::create_dir_all(&s_dir).unwrap();
            std::fs::write(s_dir.join("vmstate"), vec![0u8; 500]).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(15));
        }

        // Limit total bytes to 600 bytes -> vm-byte-1 must be evicted, keeping vm-byte-2 (500 bytes <= 600)
        let policy = SnapshotRetentionPolicy {
            max_snapshots: None,
            max_age_secs: None,
            max_total_bytes: Some(600),
        };

        let mut report = GcReport::default();
        sweep_snapshots_lru_in_dir(snap_dir, &policy, &mut report).unwrap();

        assert_eq!(report.reclaimed_snapshots, 1);
        assert_eq!(report.reclaimed_snapshot_bytes, 500);
        assert!(!snap_dir.join("vm-byte-1").exists());
        assert!(snap_dir.join("vm-byte-2").exists());
    }
}
