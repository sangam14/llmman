//! Cross-platform Copy-on-Write (Reflink) File Cloning.
//!
//! On macOS (APFS), uses `libc::clonefile`.
//! On Linux (XFS, Btrfs, ext4 with reflink), uses `ioctl(dst, FICLONE, src)`.
//! Gracefully falls back to standard file copy if reflink is not supported.

use anyhow::{Context, Result};
use std::path::Path;

/// Clones a file using copy-on-write reflink if supported by the host OS and filesystem.
///
/// Returns `Ok(true)` if a reflink clone succeeded, or `Ok(false)` if fallback copy was used.
pub fn reflink_or_copy(src: &Path, dst: &Path) -> Result<bool> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    if dst.exists() {
        let _ = std::fs::remove_file(dst);
    }

    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let src_c = CString::new(src.as_os_str().as_bytes())
            .context("Invalid source path for clonefile")?;
        let dst_c = CString::new(dst.as_os_str().as_bytes())
            .context("Invalid destination path for clonefile")?;

        let ret = unsafe { libc::clonefile(src_c.as_ptr(), dst_c.as_ptr(), 0) };
        if ret == 0 {
            return Ok(true);
        }
    }

    #[cfg(target_os = "linux")]
    {
        use std::fs::OpenOptions;
        use std::os::unix::io::AsRawFd;

        // FICLONE ioctl: _IOW(0x94, 9, int) = 0x40049409
        const FICLONE: libc::c_ulong = 0x40049409;

        if let Ok(src_file) = OpenOptions::new().read(true).open(src) {
            if let Ok(dst_file) = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(dst)
            {
                let ret =
                    unsafe { libc::ioctl(dst_file.as_raw_fd(), FICLONE, src_file.as_raw_fd()) };
                if ret == 0 {
                    return Ok(true);
                }
            }
        }
    }

    // Fallback standard copy
    std::fs::copy(src, dst).context("Failed standard file copy fallback")?;
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reflink_or_copy_basic() {
        let temp_dir = tempfile::tempdir().unwrap();
        let src = temp_dir.path().join("source.dat");
        let dst = temp_dir.path().join("clone.dat");

        std::fs::write(&src, b"test-reflink-content-12345").unwrap();

        let _reflinked = reflink_or_copy(&src, &dst).unwrap();
        assert!(dst.exists());
        assert_eq!(std::fs::read(&dst).unwrap(), b"test-reflink-content-12345");

        // Verify mutating dst does not mutate src (CoW isolation)
        std::fs::write(&dst, b"mutated-content").unwrap();
        assert_eq!(std::fs::read(&src).unwrap(), b"test-reflink-content-12345");
        assert_eq!(std::fs::read(&dst).unwrap(), b"mutated-content");
    }
}
