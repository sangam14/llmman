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

        let process = if crate::find_on_path(bin_path.to_str().unwrap_or("")).is_some()
            || bin_path.exists()
        {
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

    /// Connects to an already running Firecracker VM without spawning a new process.
    pub fn connect(socket_path: &Path) -> Self {
        Self {
            socket_path: socket_path.to_path_buf(),
            process: None,
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub fn pid(&self) -> Option<u32> {
        self.process.as_ref().and_then(|p| p.id())
    }

    // -----------------------------------------------------------------------
    // HTTP transport — single implementation for all verbs
    // -----------------------------------------------------------------------

    /// Sends an HTTP request to the Firecracker API over the Unix socket.
    /// Returns the full response body as a `String`.
    fn request(&self, method: &str, path: &str, body: Option<&str>) -> Result<String> {
        let mut stream = UnixStream::connect(&self.socket_path)
            .context("Failed to connect to Firecracker API socket")?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;

        let request = match body {
            Some(b) => format!(
                "{method} {path} HTTP/1.1\r\n\
                 Host: localhost\r\n\
                 Accept: application/json\r\n\
                 Content-Type: application/json\r\n\
                 Connection: close\r\n\
                 Content-Length: {}\r\n\
                 \r\n\
                 {b}",
                b.len()
            ),
            None => format!(
                "{method} {path} HTTP/1.1\r\n\
                 Host: localhost\r\n\
                 Accept: application/json\r\n\
                 Connection: close\r\n\
                 \r\n"
            ),
        };

        stream.write_all(request.as_bytes())?;

        let mut response = String::new();
        stream.read_to_string(&mut response)?;

        if !response.contains("HTTP/1.1 204 No Content") && !response.contains("HTTP/1.1 200 OK") {
            anyhow::bail!("API Error on {}: {}", path, response);
        }

        Ok(response)
    }

    /// Sends a JSON payload via HTTP PUT to the Firecracker API.
    fn put(&self, path: &str, body: &str) -> Result<()> {
        self.request("PUT", path, Some(body))?;
        Ok(())
    }

    /// Sends a JSON payload via HTTP PATCH to the Firecracker API.
    fn patch(&self, path: &str, body: &str) -> Result<()> {
        self.request("PATCH", path, Some(body))?;
        Ok(())
    }

    /// Sends a GET request and returns the response body.
    pub fn get(&self, path: &str) -> Result<String> {
        self.request("GET", path, None)
    }

    /// Returns `true` if the VM is responsive (GET /vm succeeds).
    pub fn is_healthy(&self) -> bool {
        self.get("/vm").is_ok()
    }

    // -----------------------------------------------------------------------
    // VM state management
    // -----------------------------------------------------------------------

    pub fn pause(&self) -> Result<()> {
        let body = serde_json::json!({"state": "Paused"}).to_string();
        self.patch("/vm", &body)
    }

    pub fn resume(&self) -> Result<()> {
        let body = serde_json::json!({"state": "Resumed"}).to_string();
        self.patch("/vm", &body)
    }

    pub fn create_snapshot(&self, snapshot_path: &Path, mem_file_path: &Path) -> Result<()> {
        let body = serde_json::json!({
            "snapshot_type": "Full",
            "snapshot_path": snapshot_path.display().to_string(),
            "mem_file_path": mem_file_path.display().to_string(),
        })
        .to_string();
        self.put("/snapshot/create", &body)
    }

    pub fn load_snapshot(&self, snapshot_path: &Path, mem_file_path: &Path) -> Result<()> {
        let body = serde_json::json!({
            "snapshot_path": snapshot_path.display().to_string(),
            "mem_backend": {
                "backend_type": "File",
                "backend_path": mem_file_path.display().to_string(),
            },
            "resume_vm": false,
        })
        .to_string();
        self.put("/snapshot/load", &body)
    }

    pub fn set_boot_source(&self, kernel_path: &Path, cmdline: &str) -> Result<()> {
        let body = serde_json::json!({
            "kernel_image_path": kernel_path.display().to_string(),
            "boot_args": cmdline,
        })
        .to_string();
        self.put("/boot-source", &body)
    }

    /// Configures the machine vCPU count and memory size (identical to agentkernel MachineConfig).
    pub fn set_machine_config(&self, vcpu_count: u32, mem_size_mib: u64) -> Result<()> {
        let body = serde_json::json!({
            "vcpu_count": vcpu_count,
            "mem_size_mib": mem_size_mib,
        })
        .to_string();
        self.put("/machine-config", &body)
    }

    /// Attaches a virtio-vsock device for guest-to-host IPC.
    pub fn set_vsock(&self, guest_cid: u32, uds_path: &Path) -> Result<()> {
        let body = serde_json::json!({
            "guest_cid": guest_cid,
            "uds_path": uds_path.display().to_string(),
        })
        .to_string();
        self.put("/vsock", &body)
    }

    pub fn set_rootfs(&self, drive_path: &Path, is_root: bool) -> Result<()> {
        let body = serde_json::json!({
            "drive_id": "rootfs",
            "path_on_host": drive_path.display().to_string(),
            "is_root_device": is_root,
            "is_read_only": false,
        })
        .to_string();
        self.put("/drives/rootfs", &body)
    }

    pub fn add_network_interface(
        &self,
        iface_id: &str,
        host_dev_name: &str,
        mac: &str,
    ) -> Result<()> {
        let body = serde_json::json!({
            "iface_id": iface_id,
            "host_dev_name": host_dev_name,
            "guest_mac": mac,
        })
        .to_string();
        self.put(&format!("/network-interfaces/{}", iface_id), &body)
    }

    pub fn start(&self) -> Result<()> {
        let body = serde_json::json!({"action_type": "InstanceStart"}).to_string();
        self.put("/actions", &body)
    }

    /// Consumes the FirecrackerVm and returns the underlying tokio Child process.
    pub fn into_inner(mut self) -> Option<Child> {
        self.process.take()
    }
}

impl Drop for FirecrackerVm {
    fn drop(&mut self) {
        if let Some(mut child) = self.process.take() {
            // Best-effort synchronous kill. `start_kill()` sends SIGKILL
            // on Unix without requiring an async runtime.
            let _ = child.start_kill();
        }
        // Clean up the socket file so a subsequent spawn on the same path
        // does not need to race against the dead process.
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
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
                            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: 20\r\n\r\n{\"state\": \"Running\"}"
                        } else {
                            "HTTP/1.1 204 No Content\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
                        };
                        let _ = s.write_all(resp.as_bytes());
                        let _ = s.shutdown(std::net::Shutdown::Both);
                    }
                }
            }
            Err(_) => break,
        }
    }
    Ok(())
}
