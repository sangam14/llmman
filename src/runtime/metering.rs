//! Append-Only Lifecycle & Compute Metering for MicroVMs.
//!
//! Inspired by Cocoon's structured lifecycle accounting, this module emits
//! durable, append-only JSONL events capturing exact compute, memory, and
//! storage usage for tenant attribution, cost auditing, and capacity planning.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Identifies the lifecycle event boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    VmComputeStart,
    VmComputeStop,
    VmStorageStart,
    VmStorageStop,
    SnapStorageStart,
    SnapStorageStop,
    InferenceRequest,
}

/// Explains why the lifecycle event was emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventReason {
    Boot,
    Clone,
    Restore,
    Hibernate,
    Resume,
    StopUser,
    StopCrash,
    GcEvict,
}

/// Hardware resource footprint at the moment the event is emitted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceShape {
    pub vcpus: u32,
    pub memory_bytes: u64,
    pub storage_bytes: u64,
}

/// A single immutable lifecycle event record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeteringEntry {
    pub kind: EventKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vm_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub reason: EventReason,
    pub hypervisor: String,
    pub shape: ResourceShape,
    pub emitted_at: DateTime<Utc>,
}

impl MeteringEntry {
    pub fn new_vm_start(
        vm_id: &str,
        model: &str,
        vcpus: u32,
        memory_mb: u64,
        storage_mb: u64,
        hypervisor: &str,
        reason: EventReason,
    ) -> Self {
        Self {
            kind: EventKind::VmComputeStart,
            vm_id: Some(vm_id.to_string()),
            snapshot_id: None,
            model: Some(model.to_string()),
            reason,
            hypervisor: hypervisor.to_string(),
            shape: ResourceShape {
                vcpus,
                memory_bytes: memory_mb * 1024 * 1024,
                storage_bytes: storage_mb * 1024 * 1024,
            },
            emitted_at: Utc::now(),
        }
    }

    pub fn new_vm_stop(
        vm_id: &str,
        model: &str,
        vcpus: u32,
        memory_mb: u64,
        storage_mb: u64,
        hypervisor: &str,
        reason: EventReason,
    ) -> Self {
        Self {
            kind: EventKind::VmComputeStop,
            vm_id: Some(vm_id.to_string()),
            snapshot_id: None,
            model: Some(model.to_string()),
            reason,
            hypervisor: hypervisor.to_string(),
            shape: ResourceShape {
                vcpus,
                memory_bytes: memory_mb * 1024 * 1024,
                storage_bytes: storage_mb * 1024 * 1024,
            },
            emitted_at: Utc::now(),
        }
    }

    pub fn new_snapshot_start(
        snapshot_id: &str,
        vm_id: &str,
        size_bytes: u64,
        reason: EventReason,
    ) -> Self {
        Self {
            kind: EventKind::SnapStorageStart,
            vm_id: Some(vm_id.to_string()),
            snapshot_id: Some(snapshot_id.to_string()),
            model: None,
            reason,
            hypervisor: "firecracker".to_string(),
            shape: ResourceShape {
                vcpus: 0,
                memory_bytes: 0,
                storage_bytes: size_bytes,
            },
            emitted_at: Utc::now(),
        }
    }

    pub fn new_snapshot_stop(snapshot_id: &str, size_bytes: u64, reason: EventReason) -> Self {
        Self {
            kind: EventKind::SnapStorageStop,
            vm_id: None,
            snapshot_id: Some(snapshot_id.to_string()),
            model: None,
            reason,
            hypervisor: "firecracker".to_string(),
            shape: ResourceShape {
                vcpus: 0,
                memory_bytes: 0,
                storage_bytes: size_bytes,
            },
            emitted_at: Utc::now(),
        }
    }
}

/// Thread-safe append-only file logger for metering events.
pub struct MeteringRecorder {
    path: PathBuf,
    file: Mutex<Option<File>>,
}

impl MeteringRecorder {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            file: Mutex::new(None),
        }
    }
}

impl Default for MeteringRecorder {
    fn default() -> Self {
        Self::new(Self::default_path())
    }
}

impl MeteringRecorder {
    /// Default metering log path inside llmman home.
    pub fn default_path() -> PathBuf {
        if let Some(home) = dirs::data_dir() {
            home.join("llmman").join("metering").join("events.jsonl")
        } else {
            PathBuf::from("/tmp/llmman/metering/events.jsonl")
        }
    }

    /// Appends one entry to the log file.
    pub fn record(&self, entry: &MeteringEntry) -> Result<()> {
        let serialized =
            serde_json::to_string(entry).context("Failed to serialize metering entry")?;
        let mut guard = self
            .file
            .lock()
            .map_err(|_| anyhow::anyhow!("Mutex poisoned"))?;

        if guard.is_none() {
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let f = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
                .with_context(|| {
                    format!("Failed to open metering log at {}", self.path.display())
                })?;
            *guard = Some(f);
        }

        if let Some(ref mut f) = *guard {
            writeln!(f, "{serialized}")?;
            f.flush()?;
        }
        Ok(())
    }

    /// Reads all entries from the log file.
    pub fn read_all(&self) -> Result<Vec<MeteringEntry>> {
        read_entries_from_path(&self.path)
    }
}

static GLOBAL_RECORDER: std::sync::LazyLock<MeteringRecorder> =
    std::sync::LazyLock::new(|| MeteringRecorder::new(MeteringRecorder::default_path()));

/// Records a global lifecycle event.
pub fn record_event(entry: &MeteringEntry) {
    let _ = GLOBAL_RECORDER.record(entry);
}

/// Reads all entries from a specified file path.
pub fn read_entries_from_path(path: &Path) -> Result<Vec<MeteringEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(path)?;
    let mut entries = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            if let Ok(entry) = serde_json::from_str::<MeteringEntry>(trimmed) {
                entries.push(entry);
            }
        }
    }
    Ok(entries)
}

/// Usage aggregation report across historical metering events.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageSummary {
    pub total_events: usize,
    pub total_vcpu_seconds: f64,
    pub total_gib_seconds: f64,
    pub active_vms: usize,
    pub distinct_models: Vec<String>,
}

/// Computes cumulative compute usage from the metering log.
pub fn summarize_usage(entries: &[MeteringEntry]) -> UsageSummary {
    let mut summary = UsageSummary {
        total_events: entries.len(),
        ..Default::default()
    };

    let mut start_times = std::collections::HashMap::new();
    let mut models = std::collections::HashSet::new();

    for entry in entries {
        if let Some(ref m) = entry.model {
            models.insert(m.clone());
        }

        match entry.kind {
            EventKind::VmComputeStart => {
                if let Some(ref id) = entry.vm_id {
                    start_times.insert(id.clone(), (entry.emitted_at, entry.shape.clone()));
                }
            }
            EventKind::VmComputeStop => {
                if let Some(ref id) = entry.vm_id {
                    if let Some((start_time, shape)) = start_times.remove(id) {
                        let duration_secs =
                            (entry.emitted_at - start_time).num_milliseconds().max(0) as f64
                                / 1000.0;
                        summary.total_vcpu_seconds += duration_secs * (shape.vcpus as f64);
                        summary.total_gib_seconds += duration_secs
                            * (shape.memory_bytes as f64 / (1024.0 * 1024.0 * 1024.0));
                    }
                }
            }
            _ => {}
        }
    }

    summary.active_vms = start_times.len();
    summary.distinct_models = models.into_iter().collect();
    summary.distinct_models.sort();
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metering_entry_serialization_roundtrip() {
        let entry = MeteringEntry::new_vm_start(
            "vm-test1234",
            "llama-3-8b",
            4,
            8192,
            50,
            "firecracker",
            EventReason::Boot,
        );

        let serialized = serde_json::to_string(&entry).unwrap();
        assert!(serialized.contains("vm_compute_start"));
        assert!(serialized.contains("vm-test1234"));
        assert!(serialized.contains("llama-3-8b"));

        let deserialized: MeteringEntry = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.kind, EventKind::VmComputeStart);
        assert_eq!(deserialized.vm_id.as_deref(), Some("vm-test1234"));
        assert_eq!(deserialized.shape.vcpus, 4);
        assert_eq!(deserialized.shape.memory_bytes, 8192 * 1024 * 1024);
    }

    #[test]
    fn test_recorder_file_append_and_summary() {
        let temp_dir = tempfile::tempdir().unwrap();
        let log_path = temp_dir.path().join("events.jsonl");

        let recorder = MeteringRecorder::new(log_path.clone());

        let t0 = Utc::now();
        let t1 = t0 + chrono::Duration::seconds(10);

        let mut start = MeteringEntry::new_vm_start(
            "vm-abc",
            "mistral-7b",
            4,
            4096,
            100,
            "firecracker",
            EventReason::Boot,
        );
        start.emitted_at = t0;

        let mut stop = MeteringEntry::new_vm_stop(
            "vm-abc",
            "mistral-7b",
            4,
            4096,
            100,
            "firecracker",
            EventReason::StopUser,
        );
        stop.emitted_at = t1;

        recorder.record(&start).unwrap();
        recorder.record(&stop).unwrap();

        let entries = recorder.read_all().unwrap();
        assert_eq!(entries.len(), 2);

        let summary = summarize_usage(&entries);
        assert_eq!(summary.total_events, 2);
        assert_eq!(summary.active_vms, 0);
        assert_eq!(summary.distinct_models, vec!["mistral-7b".to_string()]);
        // 10 seconds * 4 vcpus = 40 vcpu-seconds
        assert!((summary.total_vcpu_seconds - 40.0).abs() < 0.1);
        // 10 seconds * 4 GiB = 40 GiB-seconds
        assert!((summary.total_gib_seconds - 40.0).abs() < 0.1);
    }
}
