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
    /// Hibernate a running MicroVM (scale-to-zero memory and CPU state to disk)
    Hibernate {
        #[arg(value_name = "VM_ID")]
        vm_id: String,
    },
    /// Resume a hibernated MicroVM from disk back into execution
    Resume {
        #[arg(value_name = "VM_ID")]
        vm_id: String,
    },
    /// Display append-only lifecycle and compute metering records
    Metering {
        /// Show aggregated usage summaries across instances and models
        #[arg(short, long)]
        summary: bool,
    },
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
        /// Maximum number of snapshot states to retain (LRU eviction)
        #[arg(long)]
        max_snapshots: Option<usize>,
        /// Maximum age of snapshots before eviction (e.g. "24h", "7d", or seconds "3600")
        #[arg(long)]
        max_age: Option<String>,
        /// Maximum total disk usage for snapshots in MB (e.g. 5000)
        #[arg(long)]
        max_size_mb: Option<u64>,
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
        Some(MicrovmCommand::Hibernate { vm_id }) => {
            hibernate_microvm(vm_id)?;
            Ok(())
        }
        Some(MicrovmCommand::Resume { vm_id }) => {
            resume_microvm(vm_id).await?;
            Ok(())
        }
        Some(MicrovmCommand::Metering { summary }) => {
            display_metering(*summary)?;
            Ok(())
        }
        Some(MicrovmCommand::Stop { vm_id }) => {
            stop_microvm(vm_id);
            Ok(())
        }
        Some(MicrovmCommand::Gc {
            quiet,
            max_snapshots,
            max_age,
            max_size_mb,
        }) => {
            let max_age_secs = max_age.as_deref().and_then(parse_duration);
            let max_total_bytes = max_size_mb.map(|mb| mb * 1024 * 1024);
            let policy = crate::runtime::gc::SnapshotRetentionPolicy {
                max_snapshots: *max_snapshots,
                max_age_secs,
                max_total_bytes,
            };
            run_gc(*quiet, &policy)?;
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
        "\x1b[1m{:<14} {:<8} {:<12} {:<6} {:<10} {:<15} {:<14} {:<24}\x1b[0m",
        "VM ID", "PID", "STATUS", "VCPU", "MEMORY", "IP ADDRESS", "KERNEL", "MODEL / WORKLOAD"
    );
    println!("{:-<105}", "");

    if vms.is_empty() {
        println!("No active MicroVMs.");
        println!("\nRun \x1b[1mllmman microvm run\x1b[0m to boot a new instance.");
    } else {
        for vm in vms {
            let is_hibernated = vm.status == "HIBERNATED" || vm.status == "hibernated";
            let (color_start, color_end) = if vm.status == "RUNNING" || vm.status == "running" {
                ("\x1b[1;32m", "\x1b[0m")
            } else if is_hibernated {
                ("\x1b[1;34m", "\x1b[0m")
            } else {
                ("\x1b[1;33m", "\x1b[0m")
            };
            println!(
                "{:<14} {:<8} {}{:<12}{} {:<6} {:<10} {:<15} {:<14} {:<24}",
                vm.id,
                if is_hibernated {
                    "-".to_string()
                } else {
                    vm.pid.to_string()
                },
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

fn hibernate_microvm(vm_id: &str) -> Result<()> {
    println!(
        "\x1b[1;36m[llmman]\x1b[0m Hibernating MicroVM {} (Scale-to-Zero)...",
        vm_id
    );
    let snap_path = lifecycle::hibernate_vm(vm_id)?;
    println!(
        "\x1b[1;32m✔ MicroVM {} successfully hibernated!\x1b[0m",
        vm_id
    );
    println!("  State Snapshot:      {}", snap_path.display());
    println!("  Memory Footprint:    0 MB (Hypervisor process terminated)");
    println!(
        "  Run \x1b[1mllmman microvm resume {}\x1b[0m to restore state instantaneously.",
        vm_id
    );
    Ok(())
}

async fn resume_microvm(vm_id: &str) -> Result<()> {
    println!(
        "\x1b[1;36m[llmman]\x1b[0m Resuming hibernated MicroVM {} from snapshot...",
        vm_id
    );
    let booted = lifecycle::resume_vm(vm_id).await?;
    let pid = booted.vm.pid().unwrap_or(std::process::id());
    println!(
        "\x1b[1;32m✔ MicroVM {} successfully resumed from snapshot!\x1b[0m",
        vm_id
    );
    println!("  PID:                 {}", pid);
    println!("  Status:              \x1b[1;32mRUNNING\x1b[0m");
    println!("  IP Address:          {}", booted.guest_ip);
    println!("  Workload Target:     {}", booted.model_name);
    // In background detach mode, leak the child so the resumed VM keeps executing
    let _ = booted.into_child();
    Ok(())
}

fn display_metering(summary_only: bool) -> Result<()> {
    let recorder = crate::runtime::metering::MeteringRecorder::default();
    let events = recorder.read_all().unwrap_or_default();
    let summary = crate::runtime::metering::summarize_usage(&events);
    let estimated_cost =
        summary.total_vcpu_seconds * 0.00001 + summary.total_gib_seconds * 0.000002;

    if summary_only || events.is_empty() {
        println!("\n\x1b[1;32m─── MicroVM Resource Usage & Metering Summary ──────────────────────────────────\x1b[0m");
        println!("  Total Recorded Events:      {}", summary.total_events);
        println!("  Active Running Instances:   {}", summary.active_vms);
        println!(
            "  Total vCPU Seconds:         {:.2} vCPU-s",
            summary.total_vcpu_seconds
        );
        println!(
            "  Total Memory Footprint:     {:.2} GiB-seconds",
            summary.total_gib_seconds
        );
        println!(
            "  Distinct Workload Models:   {}",
            if summary.distinct_models.is_empty() {
                "none".to_string()
            } else {
                summary.distinct_models.join(", ")
            }
        );
        println!("  Estimated Compute Cost:     ${:.4}", estimated_cost);
        println!("\x1b[1;32m────────────────────────────────────────────────────────────────────────────────\x1b[0m\n");
        return Ok(());
    }

    println!(
        "\x1b[1m{:<25} {:<15} {:<20} {:<16} {:<12} {:<6} {:<10}\x1b[0m",
        "TIMESTAMP", "VM ID", "WORKLOAD", "EVENT", "REASON", "VCPU", "MEMORY"
    );
    println!("{:-<105}", "");

    let start_idx = if events.len() > 25 {
        events.len() - 25
    } else {
        0
    };
    for event in &events[start_idx..] {
        let (color_start, color_end) = match event.kind {
            crate::runtime::metering::EventKind::VmComputeStart => ("\x1b[1;32m", "\x1b[0m"),
            crate::runtime::metering::EventKind::VmComputeStop => ("\x1b[1;31m", "\x1b[0m"),
            crate::runtime::metering::EventKind::VmStorageStart => ("\x1b[1;36m", "\x1b[0m"),
            crate::runtime::metering::EventKind::VmStorageStop => ("\x1b[1;33m", "\x1b[0m"),
            _ => ("\x1b[1;35m", "\x1b[0m"),
        };
        let ts_full = event.emitted_at.to_rfc3339();
        let ts = if ts_full.len() > 25 {
            &ts_full[..25]
        } else {
            &ts_full
        };
        let vm_id = event.vm_id.as_deref().unwrap_or("-");
        let model_full = event.model.as_deref().unwrap_or("-");
        let model = if model_full.len() > 20 {
            &model_full[..20]
        } else {
            model_full
        };
        println!(
            "{:<25} {:<15} {:<20} {}{:<16}{} {:<12} {:<6} {:<10}",
            ts,
            vm_id,
            model,
            color_start,
            format!("{:?}", event.kind),
            color_end,
            format!("{:?}", event.reason),
            event.shape.vcpus,
            format!("{} MB", event.shape.memory_bytes / 1024 / 1024),
        );
    }

    println!("\n\x1b[1;32m─── Usage Aggregation ──────────────────────────────────────────────────────────\x1b[0m");
    println!("  Total Recorded Events:      {}", summary.total_events);
    println!("  Active Running Instances:   {}", summary.active_vms);
    println!(
        "  Total vCPU Seconds:         {:.2} vCPU-s",
        summary.total_vcpu_seconds
    );
    println!(
        "  Total Memory Footprint:     {:.2} GiB-seconds",
        summary.total_gib_seconds
    );
    println!(
        "  Distinct Workload Models:   {}",
        if summary.distinct_models.is_empty() {
            "none".to_string()
        } else {
            summary.distinct_models.join(", ")
        }
    );
    println!("  Estimated Compute Cost:     ${:.4}", estimated_cost);
    println!("\x1b[1;32m────────────────────────────────────────────────────────────────────────────────\x1b[0m\n");

    Ok(())
}

fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Ok(secs) = s.parse::<u64>() {
        return Some(secs);
    }
    if let Some(num) = s.strip_suffix('s') {
        return num.parse::<u64>().ok();
    }
    if let Some(num) = s.strip_suffix('m') {
        return num.parse::<u64>().ok().map(|m| m * 60);
    }
    if let Some(num) = s.strip_suffix('h') {
        return num.parse::<u64>().ok().map(|h| h * 3600);
    }
    if let Some(num) = s.strip_suffix('d') {
        return num.parse::<u64>().ok().map(|d| d * 86400);
    }
    None
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

fn run_gc(quiet: bool, policy: &crate::runtime::gc::SnapshotRetentionPolicy) -> Result<()> {
    if !quiet {
        println!("\x1b[1;36m[llmman]\x1b[0m Sweeping orphaned MicroVM resources, stale records, leaked tap devices, and snapshots...");
    }
    let report = crate::runtime::gc::sweep_resources_with_policy(policy)?;
    if !quiet {
        println!("\x1b[1;32m✔ MicroVM Garbage Collection Complete!\x1b[0m");
        println!("  Purged Stale Records:     {}", report.purged_records);
        println!("  Pruned TAP Devices:       {}", report.cleaned_taps);
        println!("  Removed Sockets:          {}", report.removed_sockets);
        println!("  Reclaimed Snapshots:      {}", report.reclaimed_snapshots);
        if report.reclaimed_snapshot_bytes > 0 {
            println!(
                "  Reclaimed Disk Space:     {:.2} MB",
                report.reclaimed_snapshot_bytes as f64 / (1024.0 * 1024.0)
            );
        }
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
        let args = CliWrapper::try_parse_from([
            "microvm",
            "gc",
            "--quiet",
            "--max-snapshots",
            "5",
            "--max-age",
            "24h",
            "--max-size-mb",
            "1024",
        ])
        .unwrap();
        match args.cmd {
            Some(MicrovmCommand::Gc {
                quiet,
                max_snapshots,
                max_age,
                max_size_mb,
            }) => {
                assert!(quiet);
                assert_eq!(max_snapshots, Some(5));
                assert_eq!(max_age.as_deref(), Some("24h"));
                assert_eq!(max_size_mb, Some(1024));
            }
            _ => panic!("Expected Gc command with quiet=true and retention limits"),
        }
    }

    #[test]
    fn test_microvm_hibernate_resume_parse() {
        let args = CliWrapper::try_parse_from(["microvm", "hibernate", "vm-test-123"]).unwrap();
        match args.cmd {
            Some(MicrovmCommand::Hibernate { vm_id }) => assert_eq!(vm_id, "vm-test-123"),
            _ => panic!("Expected Hibernate command"),
        }

        let args2 = CliWrapper::try_parse_from(["microvm", "resume", "vm-test-123"]).unwrap();
        match args2.cmd {
            Some(MicrovmCommand::Resume { vm_id }) => assert_eq!(vm_id, "vm-test-123"),
            _ => panic!("Expected Resume command"),
        }
    }

    #[test]
    fn test_microvm_metering_parse() {
        let args = CliWrapper::try_parse_from(["microvm", "metering", "--summary"]).unwrap();
        match args.cmd {
            Some(MicrovmCommand::Metering { summary }) => assert!(summary),
            _ => panic!("Expected Metering command"),
        }
    }

    #[test]
    fn test_parse_duration_units() {
        assert_eq!(parse_duration("30"), Some(30));
        assert_eq!(parse_duration("45s"), Some(45));
        assert_eq!(parse_duration("10m"), Some(600));
        assert_eq!(parse_duration("2h"), Some(7200));
        assert_eq!(parse_duration("1d"), Some(86400));
        assert_eq!(parse_duration("invalid"), None);
    }

    #[test]
    fn test_run_gc_executes_cleanly() {
        let policy = crate::runtime::gc::SnapshotRetentionPolicy::default();
        let res = run_gc(true, &policy);
        assert!(res.is_ok());
    }
}
