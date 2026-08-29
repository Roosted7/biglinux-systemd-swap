#!/bin/bash
# Exercise the swap-file space policy against real filesystems as they fill up.
#
# Builds loopback images of each filesystem type, fills them in steps, and asks
# the daemon's own policy functions (via the spacecheck example) what it would
# decide at each level. Loopback images are used because the interesting states
# are the dangerous ones: a btrfs with no unallocated space left is not
# something to reproduce on a disk anyone cares about.
#
# Needs root for losetup and mount. Everything it creates lives in a temp
# directory and is torn down on exit, including on failure.
#
# SPDX-License-Identifier: GPL-3.0-or-later

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="$(mktemp -d /tmp/fs-margins.XXXXXX)"
SPACECHECK="$REPO/target/debug/examples/spacecheck"

# Sizes chosen to straddle the reserve bands: 2G sits under the 1G floor once a
# 512M chunk is added, 12G is where 5% is still below the floor, 200G is where
# the percentage governs. 200G is sparse, so it costs nothing until written.
SIZES_GB=("${SIZES_GB[@]:-2 12 200}")
FSTYPES=("${FSTYPES[@]:-btrfs ext4 xfs}")
FILL_STEPS=("${FILL_STEPS[@]:-0 50 80 90 95 99}")

cleanup() {
    set +e
    for mp in "$WORK"/mnt.*; do
        [ -d "$mp" ] && mountpoint -q "$mp" && umount "$mp"
    done
    for lo in $(losetup -a 2>/dev/null | grep "$WORK" | cut -d: -f1); do
        losetup -d "$lo"
    done
    rm -rf "$WORK"
}
trap cleanup EXIT

require_root() {
    [ "$(id -u)" -eq 0 ] || { echo "must run as root (losetup/mount)" >&2; exit 1; }
}

build_checker() {
    [ -x "$SPACECHECK" ] && return
    echo "building spacecheck..."
    (cd "$REPO" && cargo build --quiet --example spacecheck)
}

# Fill a filesystem to roughly N% with incompressible data, so btrfs
# compression and ext4/xfs sparse handling cannot make the fill a lie.
fill_to() {
    local mp=$1 target_pct=$2
    local total used want
    total=$(df -B1 --output=size "$mp" | tail -1)
    want=$(( total * target_pct / 100 ))
    used=$(df -B1 --output=used "$mp" | tail -1)
    local need=$(( want - used ))
    [ "$need" -le 0 ] && return 0
    mkdir -p "$mp/ballast"
    # 64M at a time so we can stop cleanly at ENOSPC rather than dying
    local i=0
    while [ "$need" -gt 0 ]; do
        dd if=/dev/urandom of="$mp/ballast/$i" bs=1M count=64 status=none 2>/dev/null || break
        need=$(( need - 64*1024*1024 ))
        i=$(( i + 1 ))
    done
    sync
}

run_one() {
    local fstype=$1 size_gb=$2
    local img="$WORK/img.$fstype.$size_gb" mp="$WORK/mnt.$fstype.$size_gb"
    mkdir -p "$mp"

    # Sparse image: a 200G case costs only what gets written.
    truncate -s "${size_gb}G" "$img"
    case "$fstype" in
        btrfs) mkfs.btrfs -q -f "$img" >/dev/null 2>&1 || return 0 ;;
        ext4)  mkfs.ext4 -q -F "$img" >/dev/null 2>&1 || return 0 ;;
        xfs)   mkfs.xfs -q -f "$img" >/dev/null 2>&1 || return 0 ;;
    esac

    local lo
    lo=$(losetup --find --show "$img")
    mount "$lo" "$mp" 2>/dev/null || { losetup -d "$lo"; return 0; }

    for pct in "${FILL_STEPS[@]}"; do
        fill_to "$mp" "$pct"
        local actual
        actual=$(df --output=pcent "$mp" | tail -1 | tr -d ' %')
        printf '%-6s %5sG  fill~%-3s%% (df %3s%%)  ' "$fstype" "$size_gb" "$pct" "$actual"
        "$SPACECHECK" "$mp" 512M 2>/dev/null || echo "spacecheck failed"
    done

    umount "$mp"
    losetup -d "$lo"
    rm -f "$img"
}

require_root
build_checker

echo "Swap-file space policy vs real filesystems"
echo "ALLOW=false means the daemon would refuse to create a 512M swap file."
echo

for fstype in ${FSTYPES[*]}; do
    command -v "mkfs.$fstype" >/dev/null || { echo "skip $fstype (no mkfs)"; continue; }
    for size in ${SIZES_GB[*]}; do
        run_one "$fstype" "$size"
    done
    echo
done
