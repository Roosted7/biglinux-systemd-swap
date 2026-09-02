// Centralised default values for all configuration keys.
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Every module reads config keys via `config.get("key").unwrap_or(DEFAULT)`.
// Having the defaults here prevents drift between autoconfig, module code,
// swap-default.conf, and the GUI.

// ── Zram ─────────────────────────────────────────────────────────────────────

// Total disksize the pool may reach. Was 125% here while autoconfig injected
// 150%, so the documented default was never the one that ran; 150% is the
// behaviour, so it is now also the default.
pub const ZRAM_SIZE: &str = "150%";

// Size of each device. The pool grows by adding devices of this size rather
// than by enlarging them, which keeps the promotion sweep's per-device cost
// flat: a sweep costs about 8ms per GB of disksize (measured), so a device
// sized at a quarter of RAM is the same number of milliseconds to scan whether
// the pool holds two of them or six.
//
// It also sets the ceiling on device count, as zram_size / zram_device_size.
// At the defaults that is 150/25 = 6.
pub const ZRAM_DEVICE_SIZE: &str = "25%";

// Devices created at startup. The rest are added only if the pool fills.
//
// Starting small is what the sizing costs argue for: the entry table is
// vzalloc'd upfront at 16 bytes per 4K page, so disksize costs 0.39% of itself
// in RAM whether or not it is ever used, and the sweep cost scales with it too.
// Two devices is a quarter of the table and a third of the sweep of the old
// fixed four, and a pool that needs more will grow into them.
pub const ZRAM_INITIAL_DEVICES: u32 = 2;

// Refuse to grow once zsmalloc holds this share of RAM. Disksize utilization
// does not measure the danger: filling a 150%-of-RAM pool costs 75% of RAM at
// a 2x ratio and 40% at 3.78x, so the physical figure is the one that has to
// be watched.
pub const ZRAM_MAX_PHYS_PERCENT: u8 = 50;

// Main algorithm: where pages end up and spend most of their life. Chosen for
// compression ratio, since most of what sits in zram is cold.
pub const ZRAM_ALG: &str = "zstd";
pub const ZRAM_ALG_LEVEL: &str = "3";

// Initial algorithm: a faster, weaker rung pages are staged in on the way in,
// so a page that is about to be faulted straight back is cheap to reach. Once
// a page has stayed idle it is recompressed into the main algorithm.
//
// Only used where the kernel supports multi-stage compression; otherwise pages
// go directly into the main algorithm. Empty disables the rung outright.
pub const ZRAM_INITIAL_ALG: &str = "lz4";
// For lz4 the level is an acceleration factor, and 1 is both the minimum the
// backend accepts and its default, so this is a no-op there. It is spelled out
// rather than left empty so that swapping the rung to zstd inherits a level
// suited to a fast rung instead of zstd's default of 3.
pub const ZRAM_INITIAL_LEVEL: &str = "1";
pub const ZRAM_PRIO: i32 = 32767;
pub const ZRAM_MAX_DEVICES: u8 = 8;
pub const ZRAM_EXPAND_THRESHOLD: u8 = 85;
pub const ZRAM_CONTRACT_THRESHOLD: u8 = 20;
pub const ZRAM_EXPAND_COOLDOWN: u64 = 10;
pub const ZRAM_CONTRACT_STABILITY: u64 = 120;
pub const ZRAM_MIN_FREE_RAM: u8 = 15;
pub const ZRAM_CHECK_INTERVAL: u64 = 5;
pub const ZRAM_EXPAND_MIN_RATIO: f64 = 2.0;

// Promotion of pages out of the initial rung into the main algorithm.
//
// A page is promoted between IDLE_AGE and IDLE_AGE + INTERVAL after its last
// access, since it has to survive the age check and then wait for the next
// sweep. The interval is therefore kept well below the age, so the age roughly
// means what it says: here, promotion lands between 10 and 11 minutes.
//
// Sweeping often is the cheap end of the trade. It does not change how much
// recompression happens, as each page is promoted exactly once either way, but
// it spreads that work into small frequent batches rather than one burst per
// sweep, and the burst is what a desktop notices. What it does cost is the
// candidate scan, which walks the entry table in proportion to disksize rather
// than to how much of the device is in use.
// Minimum written since the last sweep before one is worth running. A sweep
// walks the entry table at about 8ms per GB of disksize whatever it finds, so
// scanning 23GB to promote a handful of pages is all cost and no benefit.
// Promoting 64MB moves roughly 14MB out of RAM at the measured ratios, which
// pays for a ~90ms scan; a few pages do not.
pub const ZRAM_RECOMP_MIN_BYTES: u64 = 64 * 1024 * 1024;

// Defer sweeps while PSI reports this share of the last ten seconds spent
// waiting for CPU. Promotion has no deadline, so on a contended machine with
// RAM to spare it is better to hold the memory a little longer and let the
// work happen when there are cycles going spare.
pub const ZRAM_RECOMP_MAX_CPU_PRESSURE: f64 = 20.0;

pub const ZRAM_RECOMP_IDLE_AGE: u64 = 600;
pub const ZRAM_RECOMP_INTERVAL: u64 = 60;

// ── Zswap ────────────────────────────────────────────────────────────────────

pub const ZSWAP_COMPRESSOR: &str = "zstd";
pub const ZSWAP_ZPOOL: &str = "zsmalloc";
pub const ZSWAP_MAX_POOL_PERCENT: u32 = 45;
pub const ZSWAP_SHRINKER_ENABLED: &str = "1";
pub const ZSWAP_ACCEPT_THRESHOLD: &str = "80";

// ── SwapFile ─────────────────────────────────────────────────────────────────

pub const SWAPFILE_PATH: &str = "/swapfile";
pub const SWAPFILE_CHUNK_SIZE: &str = "512M";
pub const SWAPFILE_MAX_COUNT: u32 = 28;
pub const SWAPFILE_MIN_COUNT: u32 = 1;
pub const SWAPFILE_FREE_RAM_PERC: u8 = 20;
pub const SWAPFILE_FREE_SWAP_PERC: u8 = 40;
pub const SWAPFILE_REMOVE_FREE_SWAP_PERC: u8 = 70;
pub const SWAPFILE_FREQUENCY: u32 = 1;
pub const SWAPFILE_SHRINK_THRESHOLD: u8 = 30;
pub const SWAPFILE_SAFE_HEADROOM: u8 = 40;
pub const SWAPFILE_NOCOW: &str = "1";

// Free space left on the filesystem after a swap file is created.
//
// Expressed as a percentage of the filesystem, clamped into an absolute band,
// because neither form works alone. A percentage is meaningless on a small
// filesystem (5% of 8GB does not cover one metadata chunk) and excessive on a
// large one (5% of 4TB is 200GB withheld for nothing).
//
// The floor is what btrfs needs to stay healthy: a metadata chunk is 256MiB to
// 1GiB and the global reserve is up to 512MiB, so a filesystem with less than
// that unallocated can fail writes and turn read-only while still reporting
// free space. ext4 and xfs do not fail this way, but both fragment badly near
// full, and a swap file wants extents.
//
// The cap is a judgement rather than a hard requirement: past a few GiB the
// reserve stops protecting the filesystem and starts being arbitrary.
pub const SWAPFILE_MIN_FREE_PERCENT: u64 = 5;
pub const SWAPFILE_MIN_FREE_FLOOR: u64 = 1024 * 1024 * 1024;
pub const SWAPFILE_MIN_FREE_CAP: u64 = 8 * 1024 * 1024 * 1024;

// Unallocated space btrfs must keep so it can still allocate a metadata chunk.
// This is the number statvfs cannot express: free space inside already
// allocated data chunks is reported as available while metadata starves.
pub const SWAPFILE_BTRFS_MIN_UNALLOCATED: u64 = 1024 * 1024 * 1024;

// ── MGLRU (Multi-Gen LRU) ──────────────────────────────────────────────────

pub const MGLRU_MIN_TTL_MS: u32 = 1000;
