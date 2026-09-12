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
use std::path::{Path, PathBuf};

use crate::runtime::{cni, firecracker::FirecrackerVm, kernel, pool, rootfs};

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
        // We can't destructure self because of the Drop on TempDir,
        // so we keep() it and rebuild.
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
            // Create a sentinel — the real dir is already kept.
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
    let boot_args = config
        .boot_args
        .as_deref()
        .unwrap_or(kernel::DEFAULT_BOOT_ARGS);

    fc.set_machine_config(config.vcpus, config.memory_mb)?;
    fc.set_boot_source(&resolved_kernel, boot_args)?;
    fc.set_rootfs(&rootfs_path, true)?;
    fc.add_network_interface("eth0", &tap_name, &config.guest_mac)?;

    // 5. Start
    fc.start()?;

    Ok(BootedVm {
        vm: fc,
        id: vm_id,
        tap_name,
        socket_path,
        rootfs_path,
        from_pool,
        kernel_path: resolved_kernel,
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

        return Ok(BootedVm {
            vm: fc,
            id: vm_id,
            tap_name,
            socket_path,
            rootfs_path,
            from_pool: false,
            kernel_path: resolved_kernel,
            _temp_dir: temp_dir,
        });
    }

    boot(config).await
}
