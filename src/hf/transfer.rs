//! Top-level HuggingFace transfer.
//!
//! **`docker` build**: fetches each file (plain HTTP or `hf-xet`, as
//! [`super::pull`] does) and streams it straight into a registry push,
//! never touching local disk. The registry-push protocol itself still
//! runs in Go (only containerd speaks it) — see
//! [`crate::oci::push_stream_open`]: Go creates the pipe and hands over
//! its write end's raw fd/HANDLE, so only that one integer crosses the
//! FFI boundary per file, not a callback per chunk.
//!
//! **`podman` build**: `copy.Image` works on whole images, not
//! individual blobs, so there's no per-blob streaming primitive to use.
//! Instead this pulls into a throwaway local OCI layout (via
//! [`super::pull`]) and pushes that with the existing `oci::push`.

use anyhow::{Context, Result};

use crate::oci::TransferOutcome;

/// Transfers `reference` (already stripped of any `hf://`/
/// `huggingface://` scheme prefix) directly to `destination`. Reports
/// what was pushed and the manifest digest that now sits at the
/// destination — mirrors `oci::transfer`'s own contract.
pub async fn transfer(reference: &str, destination: &str) -> Result<TransferOutcome> {
    via_temp_pull(reference, destination).await
}

/// The `podman`-build fallback: pull the whole model into a throwaway
/// local layout, then push that layout the ordinary way. `changed` is
/// always true on success — `oci::push` (podman's `copy.Image`) doesn't
/// report whether the destination actually changed, unlike the docker
/// path's real per-blob answer.

async fn via_temp_pull(reference: &str, destination: &str) -> Result<TransferOutcome> {
    let tmp = std::env::temp_dir().join(format!(
        "llmman-hf-transfer-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    // pull() stages the manifest under `reference` itself, but
    // `oci::push` resolves what to push by an *exact* ref lookup — so
    // the staged model has to also be findable under `destination`
    // before the push can find it at all.
    let result = super::pull::pull(reference, &tmp, "")
        .await
        .and_then(|()| super::oci::alias_manifest_ref(&tmp, reference, destination))
        .and_then(|()| {
            crate::oci::push(
                tmp.to_str()
                    .context("temp layout path is not valid UTF-8")?,
                destination,
            )
        })
        // Read back from the staged layout rather than re-resolving the
        // destination tag: this is the manifest that was just pushed, so
        // it is what `--sign-key` must sign.
        .and_then(|()| super::oci::read_manifest_ref(&tmp, destination))
        .map(|desc| TransferOutcome::new(true, desc.digest));
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

fn rand_suffix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64
}

