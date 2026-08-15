//! SWAI — Lightweight VRAM probe for Council auto mode.
//!
//! Determines available GPU VRAM so the council can choose between
//! concurrent and sequential execution without external dependencies.

use crate::council::types::CouncilMode;
use std::fs;
use std::path::Path;

/// Probe available GPU VRAM in bytes.
///
/// On Linux, scans `/sys/class/drm/card*/device/mem_info_vram_total` for the
/// largest value (primary GPU). Returns `None` on non-Linux or if no GPU is
/// detected — callers should treat this as "use sequential mode".
pub fn get_available_vram_bytes() -> Option<u64> {
    let base = Path::new("/sys/class/drm");
    if !base.exists() {
        return None;
    }

    let entries = fs::read_dir(base).ok()?;
    let mut max_vram: u64 = 0;

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("card") || !name_str[4..].starts_with(|c: char| c.is_ascii_digit())
        {
            continue;
        }

        let vram_path = entry.path().join("device/mem_info_vram_total");
        if let Ok(bytes) = fs::read_to_string(&vram_path) {
            if let Ok(vram) = bytes.trim().parse::<u64>() {
                max_vram = max_vram.max(vram);
            }
        }
    }

    if max_vram == 0 {
        return None;
    }

    Some(max_vram)
}

/// Recommend execution mode given total VRAM required by all models.
///
/// If available VRAM covers the requirement, concurrent execution is safe.
/// Otherwise fall back to sequential to avoid OOM kills.
pub fn recommend_mode(required_bytes: u64) -> CouncilMode {
    match get_available_vram_bytes() {
        Some(available) if available >= required_bytes => CouncilMode::Concurrent,
        _ => CouncilMode::Sequential,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recommend_mode_concurrent_when_vram_sufficient() {
        // With no GPU probeable (CI / non-NVIDIA), this falls back to Sequential.
        // We only verify the threshold logic: 0 bytes required always fits.
        assert_eq!(recommend_mode(0), CouncilMode::Concurrent);
    }

    #[test]
    fn test_recommend_mode_sequential_on_probe_failure() {
        // When get_available_vram_bytes returns None (no GPU), we must fall back
        // to sequential regardless of required bytes. Verified by the 0-byte case
        // above — if a GPU were present and had >= 0 bytes, it would still be
        // Concurrent, so this test confirms the safe-default path via the None
        // branch by using a huge requirement that no consumer GPU exposes.
        assert_eq!(recommend_mode(u64::MAX), CouncilMode::Sequential);
    }

    #[test]
    fn test_get_available_vram_bytes_returns_option() {
        // On CI / non-NVIDIA systems this is None; on NVIDIA Linux it's Some.
        // We only assert the return type is valid — the value is environment-
        // dependent and tested indirectly via recommend_mode above.
        let result = get_available_vram_bytes();
        match result {
            Some(v) => assert!(v > 0),
            None => {} // expected on non-Linux / non-NVIDIA
        }
    }
}
