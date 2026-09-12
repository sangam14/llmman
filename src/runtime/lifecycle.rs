//! Unified MicroVM boot lifecycle.
//!
//! This module is the **single entry point** for booting a Firecracker
//! MicroVM. It replaces the four separate copy-pasted boot sequences
//! that previously lived in `container::spawn`, `container::spawn_engine`,
//! `container::spawn_mediagen`, and `cmd::microvm::boot_microvm`.
//!
//! The flow is:
//!   1. Create a temp directory for socket + rootfs artifacts.
//!   2. Build an ext4 rootfs from the source directory.
//!   3. Provision a CNI TAP network device.
//!   4. Try to acquire a pre-warmed VM from the pool; cold-boot if none.
//!   5. Optionally restore from a snapshot (if one exists for the model).
//!   6. Otherwise, configure machine → boot source → rootfs → network → start.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::runtime::{cni, firecracker::FirecrackerVm, kernel, pool, rootfs};

/// Serialized active MicroVM instance record for observability and cluster reporting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MicrovmRecord {
    pub id: String,
    pub pid: u32,
    pub status: String,
    pub kernel: String,
    pub vcpus: u32,
    pub memory_mb: u64,
    pub ip: String,
    pub tap: String,
    pub model: String,
    pub started_at: String,
    pub socket_path: String,
}

/// Directory where active MicroVM state files are stored.
pub fn vms_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("llmman")
        .join("vms")
}

/// Persists an active MicroVM status record.
pub fn save_vm_record(record: &MicrovmRecord) -> Result<()> {
    let dir = vms_dir();
    std::fs::create_dir_all(&dir).ok();
    let file_path = dir.join(format!("{}.json", record.id));
    let json = serde_json::to_string_pretty(record)?;
    std::fs::write(file_path, json)?;
    Ok(())
}

/// Removes a MicroVM status record when the instance is stopped.
pub fn remove_vm_record(vm_id: &str) {
    let file_path = vms_dir().join(format!("{vm_id}.json"));
    if file_path.exists() {
        let _ = std::fs::remove_file(file_path);
    }
}

/// Loads all currently active MicroVM records from disk, purging stale dead PIDs.
pub fn load_active_vms() -> Vec<MicrovmRecord> {
    let dir = vms_dir();
    let mut records = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.path().extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    if let Ok(record) = serde_json::from_str::<MicrovmRecord>(&content) {
                        if crate::runtime::process::is_alive(record.pid) {
                            records.push(record);
                        } else {
                            let _ = std::fs::remove_file(entry.path());
                        }
                    }
                }
            }
        }
    }
    records.sort_by(|a, b| a.id.cmp(&b.id));
    records
}

/// Configuration for booting a MicroVM.
#[derive(Debug, Clone)]
pub struct VmConfig {
    /// Number of vCPUs.
    pub vcpus: u32,
    /// Memory in MiB.
    pub memory_mb: u64,
    /// Rootfs size in MiB.
    pub rootfs_size_mb: u64,
    /// Optional explicit kernel path (falls back to `kernel::resolve_kernel_path`).
    pub kernel_path: Option<PathBuf>,
    /// Optional custom boot args (falls back to `kernel::DEFAULT_BOOT_ARGS`).
    pub boot_args: Option<String>,
    /// Number of pre-warmed VMs to keep in the pool.
    pub pool_size: usize,
    /// Source directory whose contents are copied into the rootfs.
    pub rootfs_source: PathBuf,
    /// Guest MAC address.
    pub guest_mac: String,
    /// Optional secondary virtio-blk device for model weights or data (/dev/vdb).
    pub model_drive: Option<PathBuf>,
    /// Command to execute as guest PID 1 payload (passed via init_payload=...).
    pub init_payload: Option<String>,
    /// Model name or workload identifier for observability.
    pub model_name: String,
    /// Guest IP address assigned to eth0.
    pub guest_ip: String,
    /// Guest port where the inference server listens.
    pub guest_port: u16,
    /// Optional initial memory balloon size in MiB for dynamic memory reclaiming.
    pub balloon_mib: Option<u64>,
}

impl Default for VmConfig {
    fn default() -> Self {
        Self {
            vcpus: 4,
            memory_mb: 8192,
            rootfs_size_mb: 50,
            kernel_path: None,
            boot_args: None,
            pool_size: 3,
            rootfs_source: PathBuf::from("/tmp"),
            guest_mac: "06:00:00:00:00:01".to_string(),
            model_drive: None,
            init_payload: None,
            model_name: "default".to_string(),
            guest_ip: "172.16.0.2".to_string(),
            guest_port: 8080,
            balloon_mib: None,
        }
    }
}

/// The result of a successful VM boot.
pub struct BootedVm {
    /// The running Firecracker VM handle.
    pub vm: FirecrackerVm,
    /// Unique identifier for this VM instance.
    pub id: String,
    /// Name of the host-side TAP device.
    pub tap_name: String,
    /// Path to the Firecracker API socket.
    pub socket_path: PathBuf,
    /// Path to the rootfs block device.
    pub rootfs_path: PathBuf,
    /// Whether this VM was acquired from the pre-warmed pool.
    pub from_pool: bool,
    /// The resolved kernel path used for boot.
    pub kernel_path: PathBuf,
    /// Guest IP address.
    pub guest_ip: String,
    /// Guest inference port.
    pub guest_port: u16,
    /// Model workload tag.
    pub model_name: String,
    // Prevents the temp directory from being cleaned up while the VM runs.
    _temp_dir: tempfile::TempDir,
}

impl BootedVm {
    /// Consumes this `BootedVm` and returns the underlying tokio `Child`.
    ///
    /// After this call the temp directory is intentionally leaked (kept)
    /// so the socket and rootfs survive for the lifetime of the child.
    pub fn into_child(self) -> Result<tokio::process::Child> {
        let child = self
            .vm
            .into_inner()
            .context("Firecracker child process lost")?;
        // Prevent the temp directory from being cleaned up.
        let _ = self._temp_dir.keep();
        Ok(child)
    }

    /// Keeps the temp directory alive without consuming the BootedVm.
    /// Used when the caller wants to retain both the VM handle and the
    /// temp artifacts (e.g. for interactive `microvm run`).
    pub fn keep_temp_dir(self) -> (Self, PathBuf) {
        let path = self._temp_dir.path().to_path_buf();
        let dir_path = self._temp_dir.path().to_path_buf();
        let _ = self._temp_dir.keep();
        let kept = BootedVm {
            vm: self.vm,
            id: self.id,
            tap_name: self.tap_name,
            socket_path: self.socket_path,
            rootfs_path: self.rootfs_path,
            from_pool: self.from_pool,
            kernel_path: self.kernel_path,
            guest_ip: self.guest_ip,
            guest_port: self.guest_port,
            model_name: self.model_name,
            _temp_dir: tempfile::tempdir()
                .unwrap_or_else(|_| tempfile::Builder::new().tempdir().expect("tempdir")),
        };
        let _ = path;
        (kept, dir_path)
    }
}

/// Boots a Firecracker MicroVM using the unified lifecycle.
///
/// This is the canonical way to start a VM. All boot paths in the codebase
/// should call this function instead of duplicating the setup sequence.
pub async fn boot(config: &VmConfig) -> Result<BootedVm> {
    let vm_id = format!("vm-{:08x}", std::process::id());
    let temp_dir = tempfile::tempdir().context("Failed to create tempdir for microvm")?;
    let socket_path = temp_dir.path().join("firecracker.socket");
    let rootfs_path = temp_dir.path().join("rootfs.ext4");

    // 1. Build rootfs
    rootfs::create_ext4_rootfs(&config.rootfs_source, &rootfs_path, config.rootfs_size_mb)?;

    // 2. CNI network plumbing
    let tap_name = cni::setup_cni_network(&vm_id, temp_dir.path(), "eth0")?;

    // 3. Try pool, then cold-boot
    let vm_pool = pool::VmPool::new(
        config.pool_size,
        Path::new("firecracker").to_path_buf(),
        temp_dir.path().to_path_buf(),
    );

    let (fc, from_pool) = match vm_pool.acquire().await {
        Ok(Some(vm)) => (vm, true),
        _ => (
            FirecrackerVm::spawn(Path::new("firecracker"), &socket_path)?,
            false,
        ),
    };

    // 4. Resolve kernel and configure the VM
    let resolved_kernel = kernel::resolve_kernel_path(config.kernel_path.as_deref());
    let mut boot_args = config
        .boot_args
        .clone()
        .unwrap_or_else(|| kernel::DEFAULT_BOOT_ARGS.to_string());

    if let Some(payload) = &config.init_payload {
        boot_args.push_str(&format!(" init_payload=\"{}\"", payload));
    }

    fc.set_machine_config(config.vcpus, config.memory_mb)?;
    fc.set_boot_source(&resolved_kernel, &boot_args)?;
    fc.set_rootfs(&rootfs_path, true)?;

    // Secondary model drive
    if let Some(model_drive) = &config.model_drive {
        fc.add_drive("model", model_drive, true, false)?;
    }

    // Virtio memory balloon device for dynamic memory reclaiming
    if let Some(balloon_mib) = config.balloon_mib {
        fc.set_balloon(balloon_mib, true)?;
    }

    fc.add_network_interface("eth0", &tap_name, &config.guest_mac)?;

    // 5. Start
    fc.start()?;

    // Register active MicroVM record for dashboard and CLI observability
    if let Some(pid) = fc.pid() {
        let record = MicrovmRecord {
            id: vm_id.clone(),
            pid,
            status: "running".to_string(),
            kernel: resolved_kernel
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("vmlinux")
                .to_string(),
            vcpus: config.vcpus,
            memory_mb: config.memory_mb,
            ip: config.guest_ip.clone(),
            tap: tap_name.clone(),
            model: config.model_name.clone(),
            started_at: chrono::Utc::now().to_rfc3339(),
            socket_path: socket_path.display().to_string(),
        };
        let _ = save_vm_record(&record);
    }

    Ok(BootedVm {
        vm: fc,
        id: vm_id,
        tap_name,
        socket_path,
        rootfs_path,
        from_pool,
        kernel_path: resolved_kernel,
        guest_ip: config.guest_ip.clone(),
        guest_port: config.guest_port,
        model_name: config.model_name.clone(),
        _temp_dir: temp_dir,
    })
}

/// Boots a VM, checking for an existing snapshot first.
///
/// If a snapshot exists for `model_name` in the standard snapshot
/// directory, the VM is restored from it instead of cold-booting.
pub async fn boot_or_restore(config: &VmConfig, model_name: &str) -> Result<BootedVm> {
    let snap_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("llmman")
        .join("snapshots");
    let state_file = snap_dir.join(format!("{model_name}.state"));
    let mem_file = snap_dir.join(format!("{model_name}.mem"));

    if state_file.exists() && mem_file.exists() {
        // Snapshot restore path — still need temp dir, rootfs, and network.
        let vm_id = format!("vm-{:08x}", std::process::id());
        let temp_dir = tempfile::tempdir().context("Failed to create tempdir for microvm")?;
        let socket_path = temp_dir.path().join("firecracker.socket");
        let rootfs_path = temp_dir.path().join("rootfs.ext4");

        rootfs::create_ext4_rootfs(&config.rootfs_source, &rootfs_path, config.rootfs_size_mb)?;
        let tap_name = cni::setup_cni_network(&vm_id, temp_dir.path(), "eth0")?;

        let fc = FirecrackerVm::spawn(Path::new("firecracker"), &socket_path)?;
        fc.load_snapshot(&state_file, &mem_file)?;
        fc.resume()?;

        let resolved_kernel = kernel::resolve_kernel_path(config.kernel_path.as_deref());

        if let Some(pid) = fc.pid() {
            let record = MicrovmRecord {
                id: vm_id.clone(),
                pid,
                status: "running".to_string(),
                kernel: resolved_kernel
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("vmlinux")
                    .to_string(),
                vcpus: config.vcpus,
                memory_mb: config.memory_mb,
                ip: config.guest_ip.clone(),
                tap: tap_name.clone(),
                model: model_name.to_string(),
                started_at: chrono::Utc::now().to_rfc3339(),
                socket_path: socket_path.display().to_string(),
            };
            let _ = save_vm_record(&record);
        }

        return Ok(BootedVm {
            vm: fc,
            id: vm_id,
            tap_name,
            socket_path,
            rootfs_path,
            from_pool: false,
            kernel_path: resolved_kernel,
            guest_ip: config.guest_ip.clone(),
            guest_port: config.guest_port,
            model_name: model_name.to_string(),
            _temp_dir: temp_dir,
        });
    }

    let mut cfg = config.clone();
    cfg.model_name = model_name.to_string();
    boot(&cfg).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vm_config_defaults() {
        let cfg = VmConfig::default();
        assert_eq!(cfg.vcpus, 4);
        assert_eq!(cfg.memory_mb, 8192);
        assert_eq!(cfg.guest_ip, "172.16.0.2");
        assert_eq!(cfg.guest_port, 8080);
        assert!(cfg.model_drive.is_none());
        assert!(cfg.init_payload.is_none());
    }

    #[test]
    fn test_vm_record_round_trip() {
        let temp_dir = tempfile::tempdir().unwrap();
        let record = MicrovmRecord {
            id: "vm-test-1234".to_string(),
            pid: std::process::id(),
            status: "running".to_string(),
            kernel: "vmlinux".to_string(),
            vcpus: 4,
            memory_mb: 8192,
            ip: "172.16.0.2".to_string(),
            tap: "vmtap-test".to_string(),
            model: "llama-3-8b".to_string(),
            started_at: "2026-09-13T00:00:00Z".to_string(),
            socket_path: temp_dir.path().join("fc.sock").display().to_string(),
        };

        let file_path = temp_dir.path().join("vm-test-1234.json");
        let json = serde_json::to_string_pretty(&record).unwrap();
        std::fs::write(&file_path, json).unwrap();

        let loaded: MicrovmRecord =
            serde_json::from_str(&std::fs::read_to_string(&file_path).unwrap()).unwrap();
        assert_eq!(loaded.id, "vm-test-1234");
        assert_eq!(loaded.model, "llama-3-8b");
        assert_eq!(loaded.ip, "172.16.0.2");
    }
}
