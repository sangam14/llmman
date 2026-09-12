//! Automated Garbage Collection & Crash Convergence for MicroVM Resources.
//!
//! Inspired by Cocoon's lock-safe GC sweeper, this module cleans up:
//! 1. Stale VM records whose hypervisor PIDs are dead.
//! 2. Orphaned CNI TAP network devices (`vmtap-*`).
//! 3. Leaked Unix Domain Sockets from crashed or killed instances.

use anyhow::Result;
use std::path::Path;

use crate::runtime::{cni, lifecycle, process};

/// Observability report detailing resources reclaimed during a GC sweep.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcReport {
    pub purged_records: usize,
    pub cleaned_taps: usize,
    pub removed_sockets: usize,
}

impl GcReport {
    pub fn is_clean(&self) -> bool {
        self.purged_records == 0 && self.cleaned_taps == 0 && self.removed_sockets == 0
    }
}

/// Sweeps and reconciles all orphaned MicroVM resources.
pub fn sweep_orphaned_resources() -> Result<GcReport> {
    let mut report = GcReport::default();
    let dir = lifecycle::vms_dir();

    let mut active_taps = std::collections::HashSet::new();

    // 1. Reconcile VM JSON records
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(record) = serde_json::from_str::<lifecycle::MicrovmRecord>(&content) {
                        if process::is_alive(record.pid) {
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

    Ok(report)
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
        };

        // Save into lifecycle directory
        lifecycle::save_vm_record(&dead_record).unwrap();

        let report = sweep_orphaned_resources().unwrap();
        assert!(report.purged_records >= 1);
        assert!(!sock_path.exists());
    }
}
