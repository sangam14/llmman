//! Kernel image resolution for Firecracker MicroVMs.
//!
//! Provides a single canonical function to locate the vmlinux kernel
//! image, replacing the duplicated candidate lists that previously
//! existed in `container.rs` and `cmd/microvm.rs`.

use std::path::{Path, PathBuf};

/// The compiled kernel version string shipped with llmman.
pub const KERNEL_VERSION: &str = "6.18.45-agentkernel";

/// Human-readable kernel identification line for boot console output.
pub const KERNEL_BANNER: &str = "Linux 6.18.45-agentkernel (AgentKernel VirtIO Minimal)";

/// Default kernel boot arguments for Firecracker MicroVMs.
pub const DEFAULT_BOOT_ARGS: &str = "console=ttyS0 reboot=k panic=1 pci=off \
    root=/dev/vda rw init=/init quiet loglevel=4 i8042.nokbd i8042.noaux";

/// Ordered candidate paths for the vmlinux kernel image.
///
/// Searched top-to-bottom; the first existing file wins. The list covers:
///   1. `LLMMAN_KERNEL` env override (checked separately)
///   2. In-repo packaging directory (development)
///   3. Repo-relative path (development, different CWD)
///   4. Standard install location
///   5. Temp/CI download locations
///   6. In-repo images directory
const KERNEL_CANDIDATES: &[&str] = &[
    // Development: repo checkout
    "packaging/kernel/vmlinux-6.18.45-agentkernel",
    // System-wide install
    "/var/lib/llmman/vmlinux-6.18.45-agentkernel",
    // CI / temp download (versioned)
    "/tmp/llmman-kernel/vmlinux-6.18.45-agentkernel",
    // CI / temp download (unversioned fallback)
    "/tmp/llmman-kernel/vmlinux",
    // Alternative system location
    "/var/lib/llmman/vmlinux",
    // In-repo images directory
    "images/kernel/vmlinux-6.18.45-agentkernel",
];

/// Resolves the path to the vmlinux kernel image.
///
/// Resolution order:
///   1. `LLMMAN_KERNEL` environment variable (if set and the file exists)
///   2. The caller-provided `hint` (e.g. from `--kernel` CLI arg)
///   3. Each path in [`KERNEL_CANDIDATES`]
///   4. Falls back to `/tmp/llmman-kernel/vmlinux` (may not exist)
pub fn resolve_kernel_path(hint: Option<&Path>) -> PathBuf {
    // 1. Env override takes priority.
    if let Ok(env_path) = std::env::var("LLMMAN_KERNEL") {
        let p = PathBuf::from(env_path.trim());
        if p.exists() {
            return p;
        }
    }

    // 2. CLI-supplied hint.
    if let Some(h) = hint {
        if h.exists() {
            return h.to_path_buf();
        }
    }

    // 3. Walk the candidate list.
    for candidate in KERNEL_CANDIDATES {
        let p = Path::new(candidate);
        if p.exists() {
            return p.to_path_buf();
        }
    }

    // 4. Fallback (may not exist yet — caller will get a clear error from
    //    Firecracker's set_boot_source).
    PathBuf::from("/tmp/llmman-kernel/vmlinux")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_returns_fallback_when_nothing_exists() {
        // With no env var, no hint, and no candidate on disk, the fallback is returned.
        let path = resolve_kernel_path(None);
        assert_eq!(path, PathBuf::from("/tmp/llmman-kernel/vmlinux"));
    }

    #[test]
    fn resolve_prefers_existing_hint_over_fallback() {
        // When the hint file exists, it should be returned.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = resolve_kernel_path(Some(tmp.path()));
        assert_eq!(path, tmp.path());
    }

    #[test]
    fn kernel_version_matches_candidates() {
        // Sanity: every versioned candidate contains the version string.
        for c in KERNEL_CANDIDATES {
            if c.contains("agentkernel") {
                assert!(c.contains(KERNEL_VERSION), "{c} missing version");
            }
        }
    }
}
