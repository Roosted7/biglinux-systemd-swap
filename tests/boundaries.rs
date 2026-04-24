//! Boundary / edge-case tests for numeric inputs and decision thresholds.
//!
//! These exercise the exact "off by one" points where behaviour changes —
//! e.g. autoconfig free-disk threshold, parse_size overflow protection,
//! clamp boundaries in `SwapFileConfig` / `ZramPoolConfig`.
// SPDX-License-Identifier: GPL-3.0-or-later

use systemd_swap::autoconfig::{RecommendedConfig, SwapMode, SystemCapabilities};
use systemd_swap::config::Config;
use systemd_swap::helpers::{parse_size, GB, KB, MB};
use systemd_swap::swapfile::SwapFileConfig;
use systemd_swap::zram::ZramPoolConfig;

// ── Autoconfig decision boundaries ──────────────────────────────────────────

fn caps(fstype: Option<&str>, ram: u64, disk: u64, live: bool) -> SystemCapabilities {
    SystemCapabilities {
        swap_path_fstype: fstype.map(String::from),
        free_disk_space_bytes: disk,
        total_ram_bytes: ram,
        is_live_system: live,
        cpu_count: 4,
    }
}

#[test]
fn disk_exactly_equal_to_ram_uses_swapfc() {
    // Boundary: free_disk == ram → NOT less than → uses swapfc
    let c = RecommendedConfig::from_capabilities(&caps(Some("btrfs"), 8 * GB, 8 * GB, false));
    assert_eq!(c.swap_mode, SwapMode::ZramSwapfc);
}

#[test]
fn disk_one_byte_below_ram_uses_zram_only() {
    let c = RecommendedConfig::from_capabilities(&caps(Some("btrfs"), 8 * GB, 8 * GB - 1, false));
    assert_eq!(c.swap_mode, SwapMode::ZramOnly);
}

#[test]
fn zero_ram_edge_case_does_not_panic() {
    // ram=0 with any disk: disk (u64) >= 0 always true → passes threshold →
    // would use swapfc if FS supports. Must not divide by zero anywhere.
    let c = RecommendedConfig::from_capabilities(&caps(Some("btrfs"), 0, GB, false));
    // Either outcome is acceptable; we just require non-panic and a valid variant
    assert!(matches!(c.swap_mode, SwapMode::ZramOnly | SwapMode::ZramSwapfc));
}

#[test]
fn live_system_overrides_disk_threshold() {
    // Plenty of disk, supported FS, but live → still zram_only
    let c = RecommendedConfig::from_capabilities(&caps(Some("btrfs"), GB, 100 * GB, true));
    assert_eq!(c.swap_mode, SwapMode::ZramOnly);
}

#[test]
fn unsupported_fs_overrides_disk_threshold() {
    let c = RecommendedConfig::from_capabilities(&caps(Some("ntfs"), GB, 100 * GB, false));
    assert_eq!(c.swap_mode, SwapMode::ZramOnly);
}

// ── parse_size edge cases ───────────────────────────────────────────────────

#[test]
fn parse_size_zero_bytes() {
    assert_eq!(parse_size("0").unwrap(), 0);
}

#[test]
fn parse_size_zero_with_suffix() {
    assert_eq!(parse_size("0M").unwrap(), 0);
    assert_eq!(parse_size("0G").unwrap(), 0);
}

#[test]
fn parse_size_one_of_each_unit() {
    assert_eq!(parse_size("1K").unwrap(), KB);
    assert_eq!(parse_size("1M").unwrap(), MB);
    assert_eq!(parse_size("1G").unwrap(), GB);
    assert_eq!(parse_size("1T").unwrap(), 1024 * GB);
}

#[test]
fn parse_size_percent_zero_is_zero() {
    assert_eq!(parse_size("0%").unwrap(), 0);
}

#[test]
fn parse_size_percent_hundred_is_total_ram() {
    let ram = systemd_swap::meminfo::get_ram_size().unwrap();
    assert_eq!(parse_size("100%").unwrap(), ram);
}

#[test]
fn parse_size_percent_150_is_one_and_a_half_ram() {
    let ram = systemd_swap::meminfo::get_ram_size().unwrap();
    let expected = ram * 150 / 100;
    assert_eq!(parse_size("150%").unwrap(), expected);
}

#[test]
fn parse_size_float_errors() {
    assert!(parse_size("1.5G").is_err());
}

#[test]
fn parse_size_leading_plus_sign_accepted() {
    // `u64::from_str` accepts leading '+', so this works by design.
    assert_eq!(parse_size("+10M").unwrap(), 10 * MB);
}

// ── SwapFileConfig clamp boundaries ─────────────────────────────────────────

fn cfg(pairs: &[(&str, &str)]) -> Config {
    Config::from_pairs_for_tests(pairs.iter().map(|(k, v)| (*k, *v)))
}

#[test]
fn swapfile_max_count_exactly_28_passes() {
    let c = cfg(&[("swapfile_max_count", "28")]);
    assert_eq!(SwapFileConfig::from_config(&c).unwrap().max_count, 28);
}

#[test]
fn swapfile_max_count_29_clamps_to_28() {
    let c = cfg(&[("swapfile_max_count", "29")]);
    assert_eq!(SwapFileConfig::from_config(&c).unwrap().max_count, 28);
}

#[test]
fn swapfile_shrink_threshold_boundary_10() {
    let c = cfg(&[("swapfile_shrink_threshold", "10")]);
    assert_eq!(SwapFileConfig::from_config(&c).unwrap().shrink_threshold, 10);
}

#[test]
fn swapfile_shrink_threshold_boundary_50() {
    let c = cfg(&[("swapfile_shrink_threshold", "50")]);
    assert_eq!(SwapFileConfig::from_config(&c).unwrap().shrink_threshold, 50);
}

#[test]
fn swapfile_safe_headroom_boundary_20_60() {
    let c = cfg(&[("swapfile_safe_headroom", "20")]);
    assert_eq!(SwapFileConfig::from_config(&c).unwrap().safe_headroom, 20);

    let c = cfg(&[("swapfile_safe_headroom", "60")]);
    assert_eq!(SwapFileConfig::from_config(&c).unwrap().safe_headroom, 60);
}

#[test]
fn swapfile_chunk_min_boundary_non_sparse() {
    // Exactly 512MiB must pass through unchanged
    let c = cfg(&[("swapfile_chunk_size", "512M")]);
    let sc = SwapFileConfig::from_config(&c).unwrap();
    assert_eq!(sc.chunk_size, 512 * MB);
}

#[test]
fn swapfile_chunk_above_min_non_sparse_preserved() {
    let c = cfg(&[("swapfile_chunk_size", "2G")]);
    let sc = SwapFileConfig::from_config(&c).unwrap();
    assert_eq!(sc.chunk_size, 2 * GB);
}

#[test]
fn swapfile_chunk_below_min_sparse_clamped() {
    // Sparse min is 128MiB; below clamps up
    let c = cfg(&[
        ("swapfile_chunk_size", "32M"),
        ("swapfile_sparse_loop", "yes"),
    ]);
    let sc = SwapFileConfig::from_config(&c).unwrap();
    assert_eq!(sc.chunk_size, 128 * MB);
}

// ── ZramPoolConfig clamp boundaries ─────────────────────────────────────────

#[test]
fn pool_max_devices_exactly_8() {
    let c = cfg(&[("zram_max_devices", "8")]);
    assert_eq!(ZramPoolConfig::from_config(&c).max_devices, 8);
}

#[test]
fn pool_check_interval_min_3() {
    let c = cfg(&[("zram_check_interval", "1")]);
    assert_eq!(ZramPoolConfig::from_config(&c).check_interval, 3);
}

#[test]
fn pool_check_interval_max_300() {
    let c = cfg(&[("zram_check_interval", "9999")]);
    assert_eq!(ZramPoolConfig::from_config(&c).check_interval, 300);
}

#[test]
fn pool_contract_stability_boundary() {
    let c = cfg(&[("zram_contract_stability", "1")]);
    assert_eq!(ZramPoolConfig::from_config(&c).contract_stability, 30);

    let c = cfg(&[("zram_contract_stability", "99999")]);
    assert_eq!(ZramPoolConfig::from_config(&c).contract_stability, 600);
}

#[test]
fn pool_min_free_ram_clamps_5_40() {
    let c = cfg(&[("zram_min_free_ram", "1")]);
    assert_eq!(ZramPoolConfig::from_config(&c).min_free_ram_percent, 5);

    let c = cfg(&[("zram_min_free_ram", "99")]);
    assert_eq!(ZramPoolConfig::from_config(&c).min_free_ram_percent, 40);
}

// ── Shutdown flag (global, end-of-suite only) ───────────────────────────────
//
// SHUTDOWN is a process-wide static. Tests that touch it must run in isolation
// to avoid bleeding state into the rest of the test suite, so we keep this
// minimal and verify idempotent semantics only.

#[test]
fn is_shutdown_initial_state_or_set() {
    // Must not panic. We can observe current state but not reset.
    let _ = systemd_swap::is_shutdown();
}

#[test]
fn request_shutdown_is_idempotent() {
    systemd_swap::request_shutdown();
    assert!(systemd_swap::is_shutdown());
    systemd_swap::request_shutdown(); // double-call must not panic
    assert!(systemd_swap::is_shutdown());
}
