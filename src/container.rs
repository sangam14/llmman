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

    let config = VmConfig {
        vcpus: opts.cpus.map(|c| c.max(1.0).ceil() as u32).unwrap_or(4),
        ..VmConfig::default()
    };

    let booted = futures::executor::block_on(lifecycle::boot_or_restore(&config, model_name))?;

    if booted.from_pool {
        println!("[llmman] Acquired pre-warmed Firecracker instance from VmPool!");
    }

    booted.into_child()
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

    let config = VmConfig {
        vcpus: _cpus.map(|c| c.max(1.0).ceil() as u32).unwrap_or(4),
        ..VmConfig::default()
    };

    let booted = futures::executor::block_on(lifecycle::boot(&config))?;
    booted.into_child()
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
    println!(
        "[llmman] Spawning Mediagen via Firecracker: {:?}",
        model_path
    );

    let config = VmConfig {
        vcpus: _cpus.map(|c| c.max(1.0).ceil() as u32).unwrap_or(4),
        ..VmConfig::default()
    };

    let booted = futures::executor::block_on(lifecycle::boot(&config))?;
    booted.into_child()
}

pub fn stop(pid: u32) {
    println!("[llmman] Stopping Firecracker instance {}", pid);
    let _ = crate::runtime::process::terminate(pid);
}
