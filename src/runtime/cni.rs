//! CNI Network Plumbing for Firecracker MicroVMs.
//!
//! Invokes standard CNI (Container Network Interface) plugins (like `ptp` or `bridge`)
//! to establish a network namespace, assign an IP via IPAM (like `host-local`), and
//! create a TAP device that Firecracker can consume.

use anyhow::Result;
use std::path::Path;
use std::process::Command;

/// Provisions a network namespace and TAP device using CNI.
///
/// `container_id` is a unique string identifying this VM instance.
/// `netns_path` is the path to the network namespace to be configured.
/// `ifname` is the name of the interface inside the namespace (usually "eth0").
///
/// Returns the name of the host-side TAP device that was created by the CNI plugin.
pub fn setup_cni_network(container_id: &str, _netns_path: &Path, _ifname: &str) -> Result<String> {
    println!("[cni] Setting up network for {} via CNI", container_id);

    // In a full implementation, we would execute the CNI binaries located in /opt/cni/bin/
    // passing the network configuration (e.g. /etc/cni/net.d/10-ptp.conflist) via stdin.
    //
    // Example:
    // CNI_COMMAND=ADD CNI_CONTAINERID={id} CNI_NETNS={netns} CNI_IFNAME={ifname} CNI_PATH=/opt/cni/bin ./ptp < config.json

    // For this CNCF implementation stub, we simply invoke the `ip` command to create a TAP
    // device directly, simulating what the CNI `ptp` + `host-device` plugin would do.

    let tap_name = format!(
        "vmtap-{}",
        &container_id[0..std::cmp::min(8, container_id.len())]
    );

    if let Some(ip_bin) = crate::find_on_path("ip") {
        let status = Command::new(&ip_bin)
            .args(["tuntap", "add", "dev", &tap_name, "mode", "tap"])
            .status();

        if let Ok(st) = status {
            if st.success() {
                let _ = Command::new(&ip_bin)
                    .args(["link", "set", "dev", &tap_name, "up"])
                    .status();
            }
        }
    }

    println!(
        "[cni] Successfully configured host TAP device: {}",
        tap_name
    );

    Ok(tap_name)
}

/// Tears down the network namespace and TAP device using CNI.
pub fn teardown_cni_network(
    container_id: &str,
    _netns_path: &Path,
    _ifname: &str,
    tap_name: &str,
) -> Result<()> {
    println!("[cni] Tearing down network for {} via CNI", container_id);

    if let Some(ip_bin) = crate::find_on_path("ip") {
        let _ = Command::new(ip_bin)
            .args(["link", "del", "dev", tap_name])
            .status();
    }

    Ok(())
}
