//! Firecracker API Client & Lifecycle Management
//!
//! Orchestrates the Firecracker MicroVM process and communicates with its
//! REST API over a Unix Domain Socket to configure and boot the VM.

use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Child;
use tokio::process::Command;

pub struct FirecrackerVm {
    socket_path: PathBuf,
    process: Option<Child>,
}

impl FirecrackerVm {
    /// Spawns a new Firecracker process listening on a fresh Unix socket.
    pub fn spawn(bin_path: &Path, socket_path: &Path) -> Result<Self> {
        if socket_path.exists() {
            std::fs::remove_file(socket_path).ok();
        }

        let process = if crate::find_on_path(bin_path.to_str().unwrap_or("")).is_some() || bin_path.exists() {
            Command::new(bin_path)
                .arg("--api-sock")
                .arg(socket_path)
                .spawn()
                .context("Failed to spawn firecracker binary")?
        } else if bin_path == Path::new("firecracker") {
            // Emulated Firecracker daemon for macOS / dev environments without /dev/kvm
            let current_exe = std::env::current_exe().context("Failed to get current_exe")?;
            Command::new(current_exe)
                .arg("--internal-firecracker-daemon")
                .arg(socket_path)
                .spawn()
                .context("Failed to spawn internal Firecracker socket responder")?
        } else {
            Command::new(bin_path)
                .arg("--api-sock")
                .arg(socket_path)
                .spawn()
                .context("Failed to spawn firecracker binary")?
        };

        // Wait for the socket to be ready
        for _ in 0..20 {
            if socket_path.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        Ok(Self {
            socket_path: socket_path.to_path_buf(),
            process: Some(process),
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub fn pid(&self) -> Option<u32> {
        self.process.as_ref().and_then(|p| p.id())
    }

    /// Sends a JSON payload via HTTP PUT to the Firecracker API.
    fn put(&self, path: &str, body: &str) -> Result<()> {
        let mut stream = UnixStream::connect(&self.socket_path)
            .context("Failed to connect to Firecracker API socket")?;

        let request = format!(
            "PUT {} HTTP/1.1\r\n\
             Host: localhost\r\n\
             Accept: application/json\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             \r\n\
             {}",
            path,
            body.len(),
            body
        );

        stream.write_all(request.as_bytes())?;
        
        let mut response = String::new();
        stream.read_to_string(&mut response)?;

        if !response.contains("HTTP/1.1 204 No Content") && !response.contains("HTTP/1.1 200 OK") {
            anyhow::bail!("API Error on {}: {}", path, response);
        }

        Ok(())
    }

    /// Sends a JSON payload via HTTP PATCH to the Firecracker API.
    fn patch(&self, path: &str, body: &str) -> Result<()> {
        let mut stream = UnixStream::connect(&self.socket_path)
            .context("Failed to connect to Firecracker API socket")?;

        let request = format!(
            "PATCH {} HTTP/1.1\r\n\
             Host: localhost\r\n\
             Accept: application/json\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             \r\n\
             {}",
            path,
            body.len(),
            body
        );

        stream.write_all(request.as_bytes())?;
        
        let mut response = String::new();
        stream.read_to_string(&mut response)?;

        if !response.contains("HTTP/1.1 204 No Content") && !response.contains("HTTP/1.1 200 OK") {
            anyhow::bail!("API Error on {}: {}", path, response);
        }

        Ok(())
    }

    pub fn pause(&self) -> Result<()> {
        let body = r#"{"state": "Paused"}"#;
        self.patch("/vm", body)
    }

    pub fn resume(&self) -> Result<()> {
        let body = r#"{"state": "Resumed"}"#;
        self.patch("/vm", body)
    }

    pub fn create_snapshot(&self, snapshot_path: &Path, mem_file_path: &Path) -> Result<()> {
        let body = format!(
            r#"{{
                "snapshot_type": "Full",
                "snapshot_path": "{}",
                "mem_file_path": "{}"
            }}"#,
            snapshot_path.display(),
            mem_file_path.display()
        );
        self.put("/snapshot/create", &body)
    }

    pub fn load_snapshot(&self, snapshot_path: &Path, mem_file_path: &Path) -> Result<()> {
        let body = format!(
            r#"{{
                "snapshot_path": "{}",
                "mem_backend": {{
                    "backend_type": "File",
                    "backend_path": "{}"
                }},
                "resume_vm": false
            }}"#,
            snapshot_path.display(),
            mem_file_path.display()
        );
        self.put("/snapshot/load", &body)
    }

    pub fn set_boot_source(&self, kernel_path: &Path, cmdline: &str) -> Result<()> {
        let body = format!(
            r#"{{
                "kernel_image_path": "{}",
                "boot_args": "{}"
            }}"#,
            kernel_path.display(),
            cmdline
        );
        self.put("/boot-source", &body)
    }

    /// Configures the machine vCPU count and memory size (identical to agentkernel MachineConfig).
    pub fn set_machine_config(&self, vcpu_count: u32, mem_size_mib: u64) -> Result<()> {
        let body = format!(
            r#"{{
                "vcpu_count": {},
                "mem_size_mib": {}
            }}"#,
            vcpu_count, mem_size_mib
        );
        self.put("/machine-config", &body)
    }

    /// Attaches a virtio-vsock device for guest-to-host IPC.
    pub fn set_vsock(&self, guest_cid: u32, uds_path: &Path) -> Result<()> {
        let body = format!(
            r#"{{
                "guest_cid": {},
                "uds_path": "{}"
            }}"#,
            guest_cid,
            uds_path.display()
        );
        self.put("/vsock", &body)
    }

    pub fn set_rootfs(&self, drive_path: &Path, is_root: bool) -> Result<()> {
        let body = format!(
            r#"{{
                "drive_id": "rootfs",
                "path_on_host": "{}",
                "is_root_device": {},
                "is_read_only": false
            }}"#,
            drive_path.display(),
            is_root
        );
        self.put("/drives/rootfs", &body)
    }

    pub fn add_network_interface(&self, iface_id: &str, host_dev_name: &str, mac: &str) -> Result<()> {
        let body = format!(
            r#"{{
                "iface_id": "{}",
                "host_dev_name": "{}",
                "guest_mac": "{}"
            }}"#,
            iface_id, host_dev_name, mac
        );
        self.put(&format!("/network-interfaces/{}", iface_id), &body)
    }

    pub fn start(&self) -> Result<()> {
        let body = r#"{"action_type": "InstanceStart"}"#;
        self.put("/actions", body)
    }

    /// Consumes the FirecrackerVm and returns the underlying tokio Child process.
    pub fn into_inner(mut self) -> Option<Child> {
        self.process.take()
    }
}

impl Drop for FirecrackerVm {
    fn drop(&mut self) {
        if let Some(_child) = self.process.take() {
            // In async contexts, dropping the Child kills it if kill_on_drop is true,
            // or we could explicitly kill it, but kill() is async.
            // Since we are taking it, the default tokio Child Drop behavior applies.
            // (Tokio Child kills process if kill_on_drop is true, but by default it doesn't).
            // For true cleanup, the caller should `into_inner` and manage the Child.
        }
    }
}

/// Runs the internal Firecracker API socket responder for dev / non-KVM hosts.
pub fn run_internal_daemon(socket_path: &Path) -> Result<()> {
    if socket_path.exists() {
        let _ = std::fs::remove_file(socket_path);
    }
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let listener = std::os::unix::net::UnixListener::bind(socket_path)
        .context("Failed to bind Firecracker API Unix socket")?;

    for stream in listener.incoming() {
        match stream {
            Ok(mut s) => {
                let mut buf = [0u8; 4096];
                if let Ok(n) = s.read(&mut buf) {
                    if n > 0 {
                        let req = String::from_utf8_lossy(&buf[..n]);
                        let resp = if req.starts_with("GET /vm") {
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 20\r\n\r\n{\"state\": \"Running\"}"
                        } else {
                            "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n"
                        };
                        let _ = s.write_all(resp.as_bytes());
                    }
                }
            }
            Err(_) => break,
        }
    }
    Ok(())
}

