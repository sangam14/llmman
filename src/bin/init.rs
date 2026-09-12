//! Custom PID 1 Init for Firecracker MicroVMs.
//!
//! This minimal init system is responsible for:
//! 1. Mounting virtual filesystems (/proc, /sys, /dev).
//! 2. Bringing up local loopback and eth0.
//! 3. Spawning the requested LLM server payload.
//! 4. Reaping zombie processes so the guest doesn't run out of PIDs.
//!
//! When the payload exits, `init` exits, bringing down the MicroVM.

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    use std::process::Command;

    println!("[init] Booting llmman MicroVM...");

    mount_fs()?;
    configure_network()?;

    // Look for `init_payload=` argument on the kernel cmdline.
    let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    let payload_cmd = parse_cmdline_value(&cmdline, "init_payload").unwrap_or_else(|| {
        println!("[init] No init_payload found, defaulting to /bin/sh");
        "/bin/sh".to_string()
    });

    println!("[init] Spawning payload: {}", payload_cmd);

    let parts = split_args(&payload_cmd);
    if parts.is_empty() {
        anyhow::bail!("Empty payload command");
    }

    let mut child = Command::new(&parts[0])
        .args(&parts[1..])
        .spawn()
        .context("Failed to spawn payload")?;

    // Proper waitpid loop for PID 1 zombie reaping
    loop {
        let status = unsafe { libc::waitpid(-1, std::ptr::null_mut(), 0) };
        if status == child.id() as i32 {
            println!("[init] Main payload exited. Shutting down MicroVM.");
            break;
        }
        if status == -1 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ECHILD) {
                // No more children
                break;
            }
        }
    }

    // Attempt to sync and reboot/poweroff gracefully
    unsafe {
        libc::sync();
        libc::reboot(libc::LINUX_REBOOT_CMD_POWER_OFF);
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn mount_fs() -> anyhow::Result<()> {
    use std::ffi::CString;
    println!("[init] Mounting virtual filesystems...");

    let mounts = [
        ("proc", "/proc", "proc"),
        ("sysfs", "/sys", "sysfs"),
        ("devtmpfs", "/dev", "devtmpfs"),
    ];

    for (src, target, fs_type) in mounts {
        std::fs::create_dir_all(target).ok();

        let c_src = CString::new(src).unwrap();
        let c_target = CString::new(target).unwrap();
        let c_type = CString::new(fs_type).unwrap();

        let ret = unsafe {
            libc::mount(
                c_src.as_ptr(),
                c_target.as_ptr(),
                c_type.as_ptr(),
                0,
                std::ptr::null(),
            )
        };

        if ret != 0 {
            let err = std::io::Error::last_os_error();
            println!("[init] Warning: mount {} failed: {}", target, err);
        }
    }

    // Mount secondary virtio-blk model drive if present (/dev/vdb)
    let model_dev = std::path::Path::new("/dev/vdb");
    if model_dev.exists() {
        println!("[init] Detected secondary virtio-blk model device at /dev/vdb");
        let _ = std::fs::create_dir_all("/model");
        let c_src = CString::new("/dev/vdb").unwrap();
        let c_target = CString::new("/model").unwrap();
        let c_fs = CString::new("ext4").unwrap();
        let ret = unsafe {
            libc::mount(
                c_src.as_ptr(),
                c_target.as_ptr(),
                c_fs.as_ptr(),
                libc::MS_RDONLY,
                std::ptr::null(),
            )
        };
        if ret == 0 {
            println!("[init] Successfully mounted /dev/vdb at /model");
        } else {
            // Raw block device (e.g. unformatted raw GGUF file) - create symlink
            println!(
                "[init] /dev/vdb is not an ext4 filesystem; creating /model/model.gguf symlink"
            );
            let _ = std::os::unix::fs::symlink("/dev/vdb", "/model/model.gguf");
        }
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn configure_network() -> anyhow::Result<()> {
    use std::process::Command;
    println!("[init] Configuring network...");

    let _ = Command::new("ip")
        .args(["link", "set", "lo", "up"])
        .status();
    let _ = Command::new("ip")
        .args(["link", "set", "eth0", "up"])
        .status();

    Ok(())
}

/// Parses key=value or key="quoted value" from a Linux kernel cmdline string.
pub fn parse_cmdline_value(cmdline: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=");
    let mut search = cmdline;

    while let Some(pos) = search.find(&needle) {
        // Ensure this key starts at the beginning or is preceded by whitespace
        if pos == 0 || search[..pos].ends_with(|c: char| c.is_whitespace()) {
            let after = &search[pos + needle.len()..];
            if let Some(stripped) = after.strip_prefix('"') {
                // Quoted value
                if let Some(end) = stripped.find('"') {
                    return Some(stripped[..end].to_string());
                }
            } else if let Some(stripped) = after.strip_prefix('\'') {
                if let Some(end) = stripped.find('\'') {
                    return Some(stripped[..end].to_string());
                }
            } else {
                // Bare word up to next whitespace
                let end = after
                    .find(|c: char| c.is_whitespace())
                    .unwrap_or(after.len());
                return Some(after[..end].to_string());
            }
        }
        // Advance past this match and continue searching
        search = &search[pos + needle.len()..];
    }
    None
}

/// Splits a command string into tokens, preserving quoted arguments.
pub fn split_args(cmd: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quote = None;
    let mut chars = cmd.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '"' | '\'' if in_quote == Some(ch) => {
                in_quote = None;
            }
            '"' | '\'' if in_quote.is_none() => {
                in_quote = Some(ch);
            }
            '\\' if in_quote.is_some() => {
                if let Some(&next) = chars.peek() {
                    current.push(next);
                    chars.next();
                } else {
                    current.push('\\');
                }
            }
            c if c.is_whitespace() && in_quote.is_none() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(not(target_os = "linux"))]
fn main() -> anyhow::Result<()> {
    println!("This binary is only intended to be compiled and run inside a Linux VM.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cmdline_value() {
        let cmd = "console=ttyS0 reboot=k init_payload=\"llama-server --port 8080 -m /model/model.gguf\" quiet";
        assert_eq!(
            parse_cmdline_value(cmd, "init_payload"),
            Some("llama-server --port 8080 -m /model/model.gguf".to_string())
        );
        assert_eq!(
            parse_cmdline_value(cmd, "console"),
            Some("ttyS0".to_string())
        );
        assert_eq!(parse_cmdline_value(cmd, "quiet"), None);
        assert_eq!(parse_cmdline_value(cmd, "missing"), None);
    }

    #[test]
    fn test_split_args() {
        let cmd = "llama-server --host 0.0.0.0 --port 8080 -m \"/model/my model.gguf\"";
        let tokens = split_args(cmd);
        assert_eq!(
            tokens,
            vec![
                "llama-server",
                "--host",
                "0.0.0.0",
                "--port",
                "8080",
                "-m",
                "/model/my model.gguf"
            ]
        );
    }
}
