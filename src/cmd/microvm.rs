//! MicroVM lifecycle management, execution, and active instance monitoring.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::path::{Path, PathBuf};

use crate::runtime::{cni, kernel, lifecycle, process};

#[derive(Args, Debug)]
pub struct MicrovmArgs {
    #[command(subcommand)]
    pub command: Option<MicrovmCommand>,
}

#[derive(Subcommand, Debug)]
pub enum MicrovmCommand {
    /// Boot and run a Firecracker MicroVM with Linux kernel and rootfs
    Run {
        /// Path to uncompressed vmlinux kernel image
        #[arg(long, default_value = "/tmp/llmman-kernel/vmlinux")]
        kernel: PathBuf,
        /// Rootfs size in MB
        #[arg(long, default_value_t = 50)]
        rootfs_size_mb: u64,
        /// Number of vCPUs to assign
        #[arg(long, default_value_t = 4)]
        vcpus: u32,
        /// Memory in MB
        #[arg(long, default_value_t = 8192)]
        memory_mb: u64,
        /// Model name or workload tag
        #[arg(long, default_value = "llama-3-8b-instruct")]
        model: String,
        /// Run in background daemon mode
        #[arg(short, long)]
        detach: bool,
    },
    /// List active running MicroVM instances
    #[command(alias = "ls")]
    Ps,
    /// Inspect status and pre-warmed instances in the VmPool
    Pool,
    /// Stop a running MicroVM by ID
    Stop {
        #[arg(value_name = "VM_ID")]
        vm_id: String,
    },
    /// Reclaim orphaned MicroVM resources, stale records, leaked tap devices, and dead sockets
    Gc {
        /// Suppress non-essential output
        #[arg(short, long)]
        quiet: bool,
    },
}

pub use crate::runtime::lifecycle::{
    load_active_vms, remove_vm_record, save_vm_record, vms_dir, MicrovmRecord,
};

pub async fn run(args: &MicrovmArgs) -> Result<()> {
    match &args.command {
        Some(MicrovmCommand::Run {
            kernel,
            rootfs_size_mb,
            vcpus,
            memory_mb,
            model,
            detach,
        }) => boot_microvm(kernel, *rootfs_size_mb, *vcpus, *memory_mb, model, *detach).await,
        Some(MicrovmCommand::Ps) | None => {
            list_microvms();
            Ok(())
        }
        Some(MicrovmCommand::Pool) => {
            show_vm_pool().await;
            Ok(())
        }
        Some(MicrovmCommand::Stop { vm_id }) => {
            stop_microvm(vm_id);
            Ok(())
        }
        Some(MicrovmCommand::Gc { quiet }) => {
            run_gc(*quiet)?;
            Ok(())
        }
    }
}

async fn boot_microvm(
    kernel_hint: &Path,
    rootfs_size_mb: u64,
    vcpus: u32,
    memory_mb: u64,
    model: &str,
    detach: bool,
) -> Result<()> {
    println!("\x1b[1;36m[llmman]\x1b[0m Initializing Firecracker MicroVM Execution Environment...");

    // Use the unified lifecycle
    let config = lifecycle::VmConfig {
        vcpus,
        memory_mb,
        rootfs_size_mb,
        kernel_path: Some(kernel_hint.to_path_buf()),
        model_name: model.to_string(),
        ..lifecycle::VmConfig::default()
    };

    let booted = lifecycle::boot(&config).await?;
    let vm_id = booted.id.clone();
    let pid = booted.vm.pid().unwrap_or(std::process::id());
    let tap_name = booted.tap_name.clone();
    let active_socket = booted.socket_path.clone();
    let resolved_kernel = booted.kernel_path.clone();
    let _from_pool = booted.from_pool;
    let guest_ip = "172.16.0.2";
    let guest_mac = &config.guest_mac;

    let host_os = std::env::consts::OS;
    let host_arch = std::env::consts::ARCH;
    let hypervisor_backend =
        if cfg!(target_os = "linux") && std::path::Path::new("/dev/kvm").exists() {
            "Firecracker KVM Hypervisor (/dev/kvm)"
        } else {
            "Sandboxed MicroVM Runtime (Process & Socket Isolation)"
        };

    // Print boot output
    println!(
        "\x1b[1;32m[kernel]\x1b[0m Image: {} [{}]",
        resolved_kernel.display(),
        kernel::KERNEL_BANNER
    );
    println!("\n\x1b[1;32m─── MicroVM Runtime Telemetry ───────────────────────────────────────────────────\x1b[0m");
    println!("  Host Platform:       {} ({})", host_os, host_arch);
    println!("  Virtualization:      {}", hypervisor_backend);
    println!("  Assigned vCPUs:      {}", vcpus);
    println!("  Memory Footprint:    {} MB", memory_mb);
    println!(
        "  Rootfs Device:       virtio-blk ({} MB ext4)",
        rootfs_size_mb
    );
    println!(
        "  Network Interface:   eth0 ({}, TAP: {})",
        guest_ip, tap_name
    );
    println!("  Guest MAC:           {}", guest_mac);
    println!("  Workload Target:     {}", model);
    println!("\x1b[1;32m────────────────────────────────────────────────────────────────────────────────\x1b[0m\n");

    println!("\x1b[1;32m✔ MicroVM Instance Successfully Running!\x1b[0m");
    println!("  \x1b[1mVM ID:\x1b[0m        {}", vm_id);
    println!("  \x1b[1mPID:\x1b[0m          {}", pid);
    println!("  \x1b[1mSTATUS:\x1b[0m       \x1b[1;32mRUNNING\x1b[0m");
    println!(
        "  \x1b[1mKERNEL:\x1b[0m       Linux {}",
        kernel::KERNEL_VERSION
    );
    println!("  \x1b[1mvCPUs:\x1b[0m        {}", vcpus);
    println!("  \x1b[1mMEMORY:\x1b[0m       {} MB", memory_mb);
    println!(
        "  \x1b[1mNETWORK:\x1b[0m      eth0 ({} -> host {})",
        guest_ip, tap_name
    );
    println!("  \x1b[1mWORKLOAD:\x1b[0m     {}", model);
    println!("  \x1b[1mBACKEND:\x1b[0m      {}", hypervisor_backend);
    println!("  \x1b[1mAPI SOCKET:\x1b[0m   {}", active_socket.display());

    if !detach {
        println!(
            "\n[llmman] MicroVM running. Press Ctrl+C or run `llmman microvm stop {}` to terminate.",
            vm_id
        );
        let mut child = booted
            .into_child()
            .context("Failed to retain child process")?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\n[llmman] Shutting down MicroVM {}...", vm_id);
                let _ = child.kill().await;
                stop_microvm(&vm_id);
            }
            status = child.wait() => {
                println!("\n[llmman] MicroVM process exited: {:?}", status);
                stop_microvm(&vm_id);
            }
        }
    } else {
        // In detach mode, leak the child so the VM keeps running.
        let _ = booted.into_child();
        println!("\n[llmman] MicroVM running in background (PID {}).", pid);
    }

    Ok(())
}

fn list_microvms() {
    let _ = crate::runtime::gc::sweep_orphaned_resources();
    let vms = load_active_vms();

    println!(
        "\x1b[1m{:<14} {:<8} {:<10} {:<6} {:<10} {:<15} {:<14} {:<24}\x1b[0m",
        "VM ID", "PID", "STATUS", "VCPU", "MEMORY", "IP ADDRESS", "KERNEL", "MODEL / WORKLOAD"
    );
    println!("{:-<105}", "");

    if vms.is_empty() {
        println!("No active MicroVMs.");
        println!("\nRun \x1b[1mllmman microvm run\x1b[0m to boot a new instance.");
    } else {
        for vm in vms {
            let (color_start, color_end) = if vm.status == "RUNNING" {
                ("\x1b[1;32m", "\x1b[0m")
            } else {
                ("\x1b[1;33m", "\x1b[0m")
            };
            println!(
                "{:<14} {:<8} {}{:<10}{} {:<6} {:<10} {:<15} {:<14} {:<24}",
                vm.id,
                vm.pid,
                color_start,
                vm.status,
                color_end,
                vm.vcpus,
                format!("{} MB", vm.memory_mb),
                vm.ip,
                vm.kernel,
                vm.model
            );
        }
    }
}

async fn show_vm_pool() {
    let target = 3;
    let has_firecracker = crate::find_on_path("firecracker").is_some();
    let status_str = if has_firecracker {
        "ACTIVE (KVM Ready)"
    } else {
        "STANDBY (Local Runtime)"
    };
    let status_color = if has_firecracker { "32" } else { "36" };

    println!("\x1b[1;35mPre-Warmed MicroVM Pool (VmPool Status):\x1b[0m");
    println!("  Target Capacity:     {} instances", target);
    println!("  Warm Instances:      0 ready (demand-allocated)");
    println!("  Replenishment:       Managed on-demand via lifecycle");
    println!(
        "  Kernel Image:        Linux {} ({})",
        kernel::KERNEL_VERSION,
        kernel::resolve_kernel_path(None).display()
    );
    println!(
        "  State:               \x1b[1;{}m{}\x1b[0m",
        status_color, status_str
    );
}

fn stop_microvm(vm_id: &str) {
    crate::runtime::proxy::stop_vm_bridge(vm_id);
    let dir = vms_dir();
    let file_path = dir.join(format!("{}.json", vm_id));
    if file_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&file_path) {
            if let Ok(record) = serde_json::from_str::<MicrovmRecord>(&content) {
                let _ = process::terminate(record.pid);
                if !record.tap.is_empty() {
                    let _ = cni::teardown_cni_network(
                        &record.id,
                        Path::new("/tmp"),
                        "eth0",
                        &record.tap,
                    );
                }
                println!(
                    "[llmman] MicroVM {} (PID {}) terminated.",
                    vm_id, record.pid
                );
            }
        }
        let _ = std::fs::remove_file(file_path);
    } else {
        println!("[llmman] MicroVM {} not found or already stopped.", vm_id);
    }
}

fn run_gc(quiet: bool) -> Result<()> {
    if !quiet {
        println!("\x1b[1;36m[llmman]\x1b[0m Sweeping orphaned MicroVM resources, stale records, leaked tap devices, and sockets...");
    }
    let report = crate::runtime::gc::sweep_orphaned_resources()?;
    if !quiet {
        println!("\x1b[1;32m✔ MicroVM Garbage Collection Complete!\x1b[0m");
        println!("  Purged Stale Records: {}", report.purged_records);
        println!("  Pruned TAP Devices:   {}", report.cleaned_taps);
        println!("  Removed Sockets:      {}", report.removed_sockets);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser, Debug)]
    struct CliWrapper {
        #[command(subcommand)]
        cmd: Option<MicrovmCommand>,
    }

    #[test]
    fn test_microvm_gc_subcommand_parse() {
        let args = CliWrapper::try_parse_from(["microvm", "gc", "--quiet"]).unwrap();
        match args.cmd {
            Some(MicrovmCommand::Gc { quiet }) => assert!(quiet),
            _ => panic!("Expected Gc command with quiet=true"),
        }
    }

    #[test]
    fn test_run_gc_executes_cleanly() {
        let res = run_gc(true);
        assert!(res.is_ok());
    }
}
