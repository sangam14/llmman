//! Container execution logic swapped to Firecracker MicroVMs.
//!
//! Exposes the same public API as before (`spawn`, `spawn_engine`, `pull_image`)
//! but routes them through `crate::runtime::lifecycle` for the unified boot flow.

use anyhow::Result;
use clap::ValueEnum;
use std::path::Path;

use crate::runtime::lifecycle::{self, VmConfig};

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
    println!(
        "[llmman] {} pulling image for {:?}...",
        ociman.binary(),
        engine
    );
    Ok(())
}

pub fn spawn(
    _ociman: ContainerManager,
    model_path: &Path,
    _mmproj_path: Option<&Path>,
    _llama_cpp_version: Option<&str>,
    opts: LlamaOptions<'_>,
) -> Result<tokio::process::Child> {
    println!("[llmman] Spawning via Firecracker: {:?}", model_path);

    let model_name = model_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default");

    let guest_port = 8080;
    let payload = format!(
        "/usr/local/bin/llama-server --host 0.0.0.0 --port {guest_port} -m /model/model.gguf --ctx-size {} --threads {}",
        opts.ctx_size.unwrap_or(2048),
        opts.threads.unwrap_or(4)
    );

    let config = VmConfig {
        vcpus: opts.cpus.map(|c| c.max(1.0).ceil() as u32).unwrap_or(4),
        model_drive: Some(model_path.to_path_buf()),
        init_payload: Some(payload),
        model_name: model_name.to_string(),
        guest_port,
        ..VmConfig::default()
    };

    let booted = futures::executor::block_on(lifecycle::boot_or_restore(&config, model_name))?;

    if booted.from_pool {
        println!("[llmman] Acquired pre-warmed Firecracker instance from VmPool!");
    }

    // Start host-to-guest proxy bridge forwarding opts.port -> guest_ip:guest_port
    let bridge =
        crate::runtime::proxy::start_bridge(opts.port, &booted.guest_ip, booted.guest_port)?;
    crate::runtime::proxy::register_vm_bridge(&booted.id, bridge);

    booted.into_child()
}

pub fn spawn_engine(
    _ociman: ContainerManager,
    engine: ContainerEngine,
    model_dir: &Path,
    _version: Option<&str>,
    port: u16,
    cpus: Option<f64>,
    serve_args: impl FnOnce(&str, &str) -> Vec<String>,
) -> Result<tokio::process::Child> {
    println!("[llmman] Spawning Engine via Firecracker: {:?}", model_dir);

    let model_name = model_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default");

    let guest_port = 8080;
    let engine_bin = match engine {
        ContainerEngine::LlamaServer => "/usr/local/bin/llama-server",
        ContainerEngine::Vllm | ContainerEngine::VllmOmni => "vllm",
        ContainerEngine::Sglang => "python3 -m sglang.launch_server",
    };

    let extra_args = serve_args("/model", "0.0.0.0").join(" ");
    let payload = if extra_args.is_empty() {
        format!("{engine_bin} --host 0.0.0.0 --port {guest_port}")
    } else {
        format!("{engine_bin} --host 0.0.0.0 --port {guest_port} {extra_args}")
    };

    let config = VmConfig {
        vcpus: cpus.map(|c| c.max(1.0).ceil() as u32).unwrap_or(4),
        model_drive: Some(model_dir.to_path_buf()),
        init_payload: Some(payload),
        model_name: model_name.to_string(),
        guest_port,
        ..VmConfig::default()
    };

    let booted = futures::executor::block_on(lifecycle::boot(&config))?;

    let bridge = crate::runtime::proxy::start_bridge(port, &booted.guest_ip, booted.guest_port)?;
    crate::runtime::proxy::register_vm_bridge(&booted.id, bridge);

    booted.into_child()
}

pub fn spawn_mediagen(
    _ociman: ContainerManager,
    model_ref: &str,
    model_path: &Path,
    _cache_path: &Path,
    _version: Option<&str>,
    port: u16,
    cpus: Option<f64>,
) -> Result<tokio::process::Child> {
    println!(
        "[llmman] Spawning Mediagen via Firecracker: {:?}",
        model_path
    );

    let guest_port = 8080;
    let payload =
        format!("/usr/local/bin/llmman-mediagen --host 0.0.0.0 --port {guest_port} -m /model");

    let config = VmConfig {
        vcpus: cpus.map(|c| c.max(1.0).ceil() as u32).unwrap_or(4),
        model_drive: Some(model_path.to_path_buf()),
        init_payload: Some(payload),
        model_name: model_ref.to_string(),
        guest_port,
        ..VmConfig::default()
    };

    let booted = futures::executor::block_on(lifecycle::boot(&config))?;

    let bridge = crate::runtime::proxy::start_bridge(port, &booted.guest_ip, booted.guest_port)?;
    crate::runtime::proxy::register_vm_bridge(&booted.id, bridge);

    booted.into_child()
}

pub fn stop(pid: u32) {
    println!("[llmman] Stopping Firecracker instance {}", pid);
    for record in lifecycle::load_active_vms() {
        if record.pid == pid {
            crate::runtime::proxy::stop_vm_bridge(&record.id);
            lifecycle::remove_vm_record(&record.id);
            break;
        }
    }
    let _ = crate::runtime::process::terminate(pid);
}
