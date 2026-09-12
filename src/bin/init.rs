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

    // The command to run is either passed via kernel command line or a hardcoded fallback.
    // For now, we'll look for an `init_payload=` argument on the kernel cmdline.
    let payload_cmd = parse_kernel_cmdline_for("init_payload").unwrap_or_else(|| {
        println!("[init] No init_payload found, defaulting to /bin/sh");
        "/bin/sh".to_string()
    });

    println!("[init] Spawning payload: {}", payload_cmd);

    let parts: Vec<&str> = payload_cmd.split_whitespace().collect();
    if parts.is_empty() {
        anyhow::bail!("Empty payload command");
    }

    let mut child = Command::new(parts[0])
        .args(&parts[1..])
        .spawn()
        .context("Failed to spawn payload")?;

    // Reap zombies in a separate thread, or simply wait on the child.
    // Since this is a simple init, we wait for the main child to exit,
    // and periodically reap others (though Command::spawn handles its own child).
    // A true PID 1 would use `waitpid(-1, ...)` in a loop.

    // For CNCF robustness, we do a proper waitpid loop:
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
        std::fs::create_dir_all(target).ok(); // Ignore errors if it exists

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
            // Don't fail completely if /dev is already mounted by kernel (devtmpfs via cmdline)
            println!("[init] Warning: mount {} failed: {}", target, err);
        }
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn configure_network() -> anyhow::Result<()> {
    use std::process::Command;
    println!("[init] Configuring network...");
    // A quick hack to bring up loopback and eth0 using 'ip' or 'ifconfig'.
    // In a fully native Rust init, we would use netlink sockets (e.g. via `rtnetlink` crate),
    // but spawning `ip` is sufficient if the rootfs has iproute2.

    let _ = Command::new("ip")
        .args(["link", "set", "lo", "up"])
        .status();
    let _ = Command::new("ip")
        .args(["link", "set", "eth0", "up"])
        .status();

    Ok(())
}

#[cfg(target_os = "linux")]
fn parse_kernel_cmdline_for(key: &str) -> Option<String> {
    if let Ok(cmdline) = std::fs::read_to_string("/proc/cmdline") {
        for token in cmdline.split_whitespace() {
            if let Some(val) = token.strip_prefix(&format!("{}=", key)) {
                return Some(val.to_string());
            }
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
fn main() -> anyhow::Result<()> {
    println!("This binary is only intended to be compiled and run inside a Linux VM.");
    Ok(())
}
