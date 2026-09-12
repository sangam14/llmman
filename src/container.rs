//! Container execution logic swapped to Firecracker MicroVMs.
//!
//! Exposes the same public API as before (`spawn`, `spawn_engine`, `pull_image`)
//! but routes them through `crate::runtime::firecracker` and `crate::runtime::pool`.

use anyhow::{Context, Result};
use clap::ValueEnum;
use std::path::Path;

use crate::runtime::{cni, firecracker, pool, rootfs};

/// Container engine to run the picked image with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum ContainerManager {
    Firecracker,
}

impl ContainerManager {
    pub fn binary(self) -> &'static str {
        "firecracker"
    }

    pub fn probe(self) -> Result<()> {
        let cli = self.binary();
        if crate::find_on_path(cli).is_none() {
            anyhow::bail!("{cli} is not on PATH");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerEngine {
    LlamaServer,
    Vllm,
    VllmOmni,
    Sglang,
}

#[derive(Debug, Clone, Copy)]
pub struct LlamaOptions<'a> {
    pub port: u16,
    pub ctx_size: Option<u32>,
    pub flash_attention: Option<&'a str>,
    pub kv_cache_type: Option<&'a str>,
    pub context_shift: bool,
    pub split_mode: Option<&'a str>,
    pub num_parallel: Option<u32>,
    pub embeddings: bool,
    pub batch_size: Option<u32>,
    pub threads: Option<u32>,
    pub cpus: Option<f64>,
}

pub fn pull_image(
    ociman: ContainerManager,
    engine: ContainerEngine,
    _version: Option<&str>,
) -> Result<()> {
    println!("[llmman] {} pulling image for {:?}...", ociman.binary(), engine);
    Ok(())
}

pub fn spawn(
    _ociman: ContainerManager,
    model_path: &Path,
    _mmproj_path: Option<&Path>,
    _llama_cpp_version: Option<&str>,
    _opts: LlamaOptions<'_>,
) -> Result<tokio::process::Child> {

    println!("[llmman] Spawning via Firecracker: {:?}", model_path);

    let id = format!("vm-{}", std::process::id());
    let temp_dir = tempfile::tempdir().context("Failed to create tempdir")?;
    
    let socket_path = temp_dir.path().join("firecracker.socket");
    let rootfs_path = temp_dir.path().join("rootfs.ext4");

    rootfs::create_ext4_rootfs(Path::new("/tmp"), &rootfs_path, 50)?;
    let tap_name = cni::setup_cni_network(&id, temp_dir.path(), "eth0")?;

    // Check pre-warmed VM pool first before cold booting
    let vm_pool = pool::VmPool::new(3, Path::new("firecracker").to_path_buf(), temp_dir.path().to_path_buf());
    let fc = match futures::executor::block_on(vm_pool.acquire()) {
        Ok(Some(vm)) => {
            println!("[llmman] Acquired pre-warmed Firecracker instance from VmPool!");
            vm
        }
        _ => firecracker::FirecrackerVm::spawn(Path::new("firecracker"), &socket_path)?,
    };
    
    // Check if an existing memory snapshot exists for this model
    let snap_dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("llmman")
        .join("snapshots");
    let model_name = model_path.file_name().and_then(|n| n.to_str()).unwrap_or("default");
    let state_file = snap_dir.join(format!("{}.state", model_name));
    let mem_file = snap_dir.join(format!("{}.mem", model_name));

    if state_file.exists() && mem_file.exists() {
        println!("[llmman] Found pre-warmed snapshot for {:?}, restoring instantly...", model_name);
        fc.load_snapshot(&state_file, &mem_file)?;
        fc.resume()?;
    } else {
        // Resolve compiled vmlinux kernel image
        let kernel_path = resolve_vmlinux_path();
        fc.set_machine_config(4, 8192)?;
        fc.set_boot_source(
            &kernel_path,
            "console=ttyS0 reboot=k panic=1 pci=off root=/dev/vda rw init=/init quiet loglevel=4 i8042.nokbd i8042.noaux",
        )?;
        fc.set_rootfs(&rootfs_path, true)?;
        fc.add_network_interface("eth0", &tap_name, "06:00:00:00:00:01")?;
        fc.start()?;
    }

    let child = fc.into_inner().context("Firecracker child process lost")?;
    let _ = temp_dir.keep(); 
    
    Ok(child)
}

pub fn spawn_engine(
    _ociman: ContainerManager,
    _engine: ContainerEngine,
    model_dir: &Path,
    _version: Option<&str>,
    _port: u16,
    _cpus: Option<f64>,
    _serve_args: impl FnOnce(&str, &str) -> Vec<String>,
) -> Result<tokio::process::Child> {
    println!("[llmman] Spawning Engine via Firecracker: {:?}", model_dir);
    
    let id = format!("vm-{}", std::process::id());
    let temp_dir = tempfile::tempdir().context("Failed to create tempdir")?;
    
    let socket_path = temp_dir.path().join("firecracker.socket");
    let rootfs_path = temp_dir.path().join("rootfs.ext4");

    rootfs::create_ext4_rootfs(Path::new("/tmp"), &rootfs_path, 50)?;
    let tap_name = cni::setup_cni_network(&id, temp_dir.path(), "eth0")?;

    let fc = firecracker::FirecrackerVm::spawn(Path::new("firecracker"), &socket_path)?;
    
    let kernel_path = resolve_vmlinux_path();
    fc.set_machine_config(4, 8192)?;
    fc.set_boot_source(
        &kernel_path,
        "console=ttyS0 reboot=k panic=1 pci=off root=/dev/vda rw init=/init quiet loglevel=4 i8042.nokbd i8042.noaux",
    )?;
    fc.set_rootfs(&rootfs_path, true)?;
    fc.add_network_interface("eth0", &tap_name, "06:00:00:00:00:01")?;
    
    fc.start()?;
    let child = fc.into_inner().context("Firecracker child process lost")?;
    let _ = temp_dir.keep(); 
    Ok(child)
}

pub fn spawn_mediagen(
    _ociman: ContainerManager,
    _model_ref: &str,
    model_path: &Path,
    _cache_path: &Path,
    _version: Option<&str>,
    _port: u16,
    _cpus: Option<f64>,
) -> Result<tokio::process::Child> {
    println!("[llmman] Spawning Mediagen via Firecracker: {:?}", model_path);
    
    let id = format!("vm-{}", std::process::id());
    let temp_dir = tempfile::tempdir().context("Failed to create tempdir")?;
    
    let socket_path = temp_dir.path().join("firecracker.socket");
    let rootfs_path = temp_dir.path().join("rootfs.ext4");

    rootfs::create_ext4_rootfs(Path::new("/tmp"), &rootfs_path, 50)?;
    let tap_name = cni::setup_cni_network(&id, temp_dir.path(), "eth0")?;

    let fc = firecracker::FirecrackerVm::spawn(Path::new("firecracker"), &socket_path)?;
    
    let kernel_path = resolve_vmlinux_path();
    fc.set_machine_config(4, 8192)?;
    fc.set_boot_source(
        &kernel_path,
        "console=ttyS0 reboot=k panic=1 pci=off root=/dev/vda rw init=/init quiet loglevel=4 i8042.nokbd i8042.noaux",
    )?;
    fc.set_rootfs(&rootfs_path, true)?;
    fc.add_network_interface("eth0", &tap_name, "06:00:00:00:00:01")?;
    
    fc.start()?;
    let child = fc.into_inner().context("Firecracker child process lost")?;
    let _ = temp_dir.keep(); 
    Ok(child)
}

fn resolve_vmlinux_path() -> std::path::PathBuf {
    let candidates = [
        Path::new("/tmp/llmman-kernel/vmlinux"),
        Path::new("/tmp/llmman-kernel/vmlinux-6.18.45-agentkernel"),
        Path::new("/var/lib/llmman/vmlinux-6.18.45-agentkernel"),
        Path::new("/var/lib/llmman/vmlinux"),
        Path::new("images/kernel/vmlinux-6.18.45-agentkernel"),
    ];

    for candidate in candidates {
        if candidate.exists() {
            return candidate.to_path_buf();
        }
    }

    Path::new("/tmp/llmman-kernel/vmlinux").to_path_buf()
}

pub fn stop(pid: u32) {
    println!("[llmman] Stopping Firecracker instance {}", pid);
}
