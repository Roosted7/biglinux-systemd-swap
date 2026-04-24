//! CLI smoke tests.
//!
//! Spawns the real `systemd-swap` binary via Cargo's `CARGO_BIN_EXE_*` env var
//! and verifies end-user-observable behaviour: exit codes, help output,
//! subcommand availability, and unprivileged-mode graceful failures.
// SPDX-License-Identifier: GPL-3.0-or-later

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_systemd-swap")
}

// ── Help / version ──────────────────────────────────────────────────────────

#[test]
fn help_flag_succeeds() {
    let out = Command::new(bin()).arg("--help").output().unwrap();
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("systemd-swap"), "stdout was: {}", stdout);
    assert!(stdout.contains("start"));
    assert!(stdout.contains("stop"));
    assert!(stdout.contains("status"));
    assert!(stdout.contains("autoconfig"));
}

#[test]
fn short_help_flag_works() {
    let out = Command::new(bin()).arg("-h").output().unwrap();
    assert!(out.status.success());
}

#[test]
fn version_flag_reports_version() {
    let out = Command::new(bin()).arg("--version").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // version string should contain at least the binary name
    assert!(stdout.contains("systemd-swap"), "stdout was: {}", stdout);
}

#[test]
fn no_args_prints_help_and_exits_zero() {
    let out = Command::new(bin()).output().unwrap();
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    // main.rs prints help when no subcommand given
    assert!(stdout.contains("systemd-swap"));
}

#[test]
fn unknown_subcommand_fails_non_zero() {
    let out = Command::new(bin())
        .arg("bogus-subcommand-does-not-exist")
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn subcommand_help_works() {
    let out = Command::new(bin()).args(["start", "--help"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.to_lowercase().contains("start"));
}

// ── Read-only subcommands run without root ──────────────────────────────────

#[test]
fn status_runs_without_root() {
    let out = Command::new(bin()).arg("status").output().unwrap();
    assert!(
        out.status.success(),
        "status failed unprivileged.\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn status_output_includes_swap_section() {
    let out = Command::new(bin()).arg("status").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Must always have a Swap section (even if "none")
    assert!(stdout.contains("Swap:"), "stdout was: {}", stdout);
}

#[test]
fn autoconfig_runs_without_root() {
    let out = Command::new(bin()).arg("autoconfig").output().unwrap();
    assert!(
        out.status.success(),
        "autoconfig failed unprivileged.\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn autoconfig_reports_key_sections() {
    let out = Command::new(bin()).arg("autoconfig").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("System Information"));
    assert!(stdout.contains("Recommended Mode"));
    assert!(stdout.contains("Config Keys"));
}

#[test]
fn autoconfig_reports_zram_alg_key() {
    let out = Command::new(bin()).arg("autoconfig").output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("zram_alg"));
    assert!(stdout.contains("zram_size"));
    assert!(stdout.contains("mglru_min_ttl_ms"));
}

// ── Privileged subcommands exit cleanly when not root ───────────────────────

#[test]
fn start_without_root_exits_non_zero_without_panic() {
    let out = Command::new(bin()).arg("start").output().unwrap();
    // Unprivileged invocation should fail, not panic.
    if nix::unistd::geteuid().is_root() {
        // Skip if running as root — would actually start the service.
        return;
    }
    assert!(!out.status.success(), "start should fail unprivileged");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("panicked at"),
        "unexpected panic: {}",
        stderr
    );
    // The am_i_root() check surfaces as an ERRO log line
    assert!(stderr.contains("ERRO") || stderr.contains("root"), "stderr={}", stderr);
}

#[test]
fn stop_without_root_exits_non_zero_without_panic() {
    let out = Command::new(bin()).arg("stop").output().unwrap();
    if nix::unistd::geteuid().is_root() {
        return;
    }
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("panicked at"), "unexpected panic: {}", stderr);
}

// ── Binary metadata ─────────────────────────────────────────────────────────

#[test]
fn binary_exists_at_cargo_exe_path() {
    let path = std::path::Path::new(bin());
    assert!(path.is_file(), "binary not at {:?}", path);
    let meta = std::fs::metadata(path).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mode = meta.permissions().mode();
    assert!(mode & 0o100 != 0, "binary not executable: mode={:o}", mode);
}
