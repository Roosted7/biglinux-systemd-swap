//! Black-box integration tests against the systemd-swap library API.
//!
//! These tests exercise only the public surface of the `systemd_swap` crate —
//! they do not touch privileged sysfs/procfs paths or invoke external
//! commands.  Anything that needs root lives in src/-level unit tests gated by
//! normal Rust conditional compilation.
// SPDX-License-Identifier: GPL-3.0-or-later

use std::io::Write;

use systemd_swap::autoconfig::{RecommendedConfig, SwapMode};
use systemd_swap::config::Config;
use systemd_swap::helpers::{self, parse_size};
use systemd_swap::meminfo;
use systemd_swap::swapfile::{SwapFileConfig, SwapFileError, SwapFileInfo};
use systemd_swap::zram::{ZramPoolConfig, ZramStats};

// ── Config end-to-end ────────────────────────────────────────────────────────

#[test]
fn config_round_trip_from_pairs() {
    let cfg = Config::from_pairs_for_tests([
        ("zram_alg", "zstd"),
        ("zram_size", "150%"),
        ("swapfile_path", "/var/swap"),
    ]);
    assert_eq!(cfg.get("zram_alg").unwrap(), "zstd");
    assert_eq!(cfg.get("zram_size").unwrap(), "150%");
    assert_eq!(cfg.get_opt("missing_key"), None);
}

#[test]
fn config_from_file_parses_keys_and_comments() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(
        file,
        "# top comment\n\
         zram_alg=zstd\n\
         swapfile_max_count=10  # inline comment\n\
         \n\
         swapfile_path=/swap"
    )
    .unwrap();
    let cfg = Config::from_file_for_tests(file.path()).unwrap();
    assert_eq!(cfg.get("zram_alg").unwrap(), "zstd");
    assert_eq!(cfg.get_as::<u32>("swapfile_max_count").unwrap(), 10);
    assert_eq!(cfg.get("swapfile_path").unwrap(), "/swap");
}

#[test]
fn config_apply_autoconfig_respects_user_overrides() {
    let mut cfg = Config::from_pairs_for_tests([("zram_size", "200%")]);
    let recommended = RecommendedConfig::default(); // zram_only with 150%
    cfg.apply_autoconfig(&recommended);
    // User-defined value is kept.
    assert_eq!(cfg.get("zram_size").unwrap(), "200%");
    // Non-overridden keys are injected.
    assert_eq!(cfg.get("zram_alg").unwrap(), "zstd");
}

// ── Autoconfig → ZramPoolConfig pipeline ─────────────────────────────────────

#[test]
fn auto_injected_config_feeds_zram_pool() {
    let mut cfg = Config::from_pairs_for_tests(std::iter::empty::<(&str, &str)>());
    cfg.apply_autoconfig(&RecommendedConfig::default());
    let pool = ZramPoolConfig::from_config(&cfg);
    assert_eq!(pool.algorithm, "zstd");
    assert_eq!(pool.initial_size_percent, 150);
}

// ── Autoconfig → SwapFileConfig pipeline ─────────────────────────────────────

#[test]
fn auto_zram_swapfc_feeds_swapfile_config() {
    let mut cfg = Config::from_pairs_for_tests([("swapfile_path", "/swap")]);
    // Manually apply zram_swapfc-style pairs via public API
    let mut rec = RecommendedConfig::default();
    // Convert default (zram_only) into the swapfc shape using the public
    // autoconfig output — emulate by constructing pairs directly.
    rec = force_zram_swapfc(rec);
    cfg.apply_autoconfig(&rec);
    let sc = SwapFileConfig::from_config(&cfg).unwrap();
    assert!(sc.max_count >= 1);
    assert!(sc.chunk_size >= 128 * 1024 * 1024);
}

/// Build a zram+swapfc recommended config by toggling swap_mode and copying
/// swapfc-related defaults. Keeps test independent of private constructors.
fn force_zram_swapfc(mut rec: RecommendedConfig) -> RecommendedConfig {
    rec.swap_mode = SwapMode::ZramSwapfc;
    rec.swapfc_max_count = 28;
    rec
}

// ── parse_size across units ──────────────────────────────────────────────────

#[test]
fn parse_size_produces_monotonic_sequence() {
    let values = [
        parse_size("1K").unwrap(),
        parse_size("1M").unwrap(),
        parse_size("1G").unwrap(),
        parse_size("1T").unwrap(),
    ];
    for w in values.windows(2) {
        assert!(w[0] < w[1]);
    }
}

#[test]
fn parse_size_percent_uses_real_ram() {
    // On any machine with >0 RAM, 1% of it is positive.
    if meminfo::get_ram_size().is_ok() {
        assert!(parse_size("1%").unwrap() > 0);
        assert!(parse_size("100%").unwrap() > 0);
    }
}

// ── Invalid swapfile path rejection ──────────────────────────────────────────

#[test]
fn invalid_swapfile_path_errors() {
    let cfg = Config::from_pairs_for_tests([("swapfile_path", "/sys/kernel/tmp")]);
    assert!(matches!(
        SwapFileConfig::from_config(&cfg),
        Err(SwapFileError::InvalidPath)
    ));
}

// ── Basic meminfo is readable on any Linux host ──────────────────────────────

#[test]
fn system_reports_ram_size() {
    let ram = meminfo::get_ram_size().expect("should read MemTotal");
    assert!(ram > 0);
}

#[test]
fn system_reports_bounded_free_ram() {
    let pct = meminfo::get_free_ram_percent().expect("should read MemAvailable");
    assert!(pct <= 100);
}

// ── Helper path utilities must not panic on weird inputs ────────────────────

#[test]
fn force_remove_accepts_missing_path() {
    helpers::force_remove("/definitely/not/there/xyz.swap", false);
}

#[test]
fn get_what_from_swap_unit_smoke() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "[Swap]\nWhat=/dev/zram0").unwrap();
    assert_eq!(
        helpers::get_what_from_swap_unit(file.path()).as_deref(),
        Some("/dev/zram0")
    );
}

// ── SwapFileInfo semantics ───────────────────────────────────────────────────

#[test]
fn swap_file_info_percent_and_empty_check() {
    let f = SwapFileInfo {
        path: std::path::PathBuf::from("/swap/1"),
        size_bytes: 1000,
        used_bytes: 100,
        priority: 100,
    };
    assert_eq!(f.usage_percent(), 10);
    assert!(f.is_nearly_empty(30));
    assert!(!f.is_nearly_empty(5));
}

// ── ZramStats ratio reporting ────────────────────────────────────────────────

#[test]
fn zram_stats_compression_ratio_via_public_fields() {
    let s = ZramStats {
        orig_data_size: 1000,
        compr_data_size: 250,
        mem_used_total: 250,
        mem_limit: 0,
        disksize: 2000,
        same_pages: 0,
        pages_compacted: 0,
    };
    assert!((s.compression_ratio() - 4.0).abs() < 1e-9);
    assert_eq!(s.memory_utilization(), 50);
}
