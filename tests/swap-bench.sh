#!/bin/bash
# Drive a realistic memory load into swap and record what it costs.
#
# The load runs inside a transient cgroup with MemoryMax set, so it swaps
# against its own limit rather than the machine's. That keeps the measurement
# repeatable regardless of what else is running, and means a mistake here
# starves the benchmark instead of the desktop: if it overruns, the kernel OOMs
# the scope, not your session.
#
# Workers differ in working-set size and access rate rather than all hammering
# equally, because a uniform load never produces the mix of hot and cold pages
# the recompression rungs exist to exploit. A separate low-memory canary times
# how long its own pages take to come back, which is the closest thing to a
# responsiveness number available without instrumenting the desktop.
#
# SPDX-License-Identifier: GPL-3.0-or-later

set -euo pipefail

MEM_MAX="${MEM_MAX:-8G}"          # cgroup ceiling for the whole load
SWAP_MAX="${SWAP_MAX:-6G}"        # how much of it may spill
WORKERS="${WORKERS:-6}"
DURATION="${DURATION:-600}"       # seconds; must exceed zram_recomp_idle_age
SAMPLE="${SAMPLE:-10}"            # seconds between samples
OUT="${OUT:-/tmp/swap-bench.$(date +%s)}"

mkdir -p "$OUT"

need() { command -v "$1" >/dev/null || { echo "missing: $1" >&2; exit 1; }; }
need systemd-run
need python3

# ── The load ────────────────────────────────────────────────────────────────
# Compressible but not trivially so: repeated structured text compresses like
# real application memory, where zeros would be deduplicated as same-pages and
# random data would defeat compression entirely, and neither tells us anything.
cat > "$OUT/worker.py" <<'PY'
import os, sys, time, random

idx = int(sys.argv[1]); mb = int(sys.argv[2]); rate = float(sys.argv[3])
random.seed(idx)

vocab = [f"token{i:04d}" for i in range(256)]
def block():
    return (" ".join(random.choice(vocab) for _ in range(64)) + "\n").encode() * 256

pages = [bytearray(block()) for _ in range(mb)]

# Tiered access: a small hot set touched constantly, a warm set occasionally,
# and a cold remainder left alone so it can actually age into the idle rung.
hot  = max(1, len(pages) // 10)
warm = max(1, len(pages) //  3)

while True:
    r = random.random()
    if r < 0.80:   i = random.randrange(0, hot)
    elif r < 0.95: i = random.randrange(hot, warm)
    else:          i = random.randrange(warm, len(pages))
    pages[i][0] = (pages[i][0] + 1) % 256
    time.sleep(rate)
PY

# ── The canary ──────────────────────────────────────────────────────────────
# Tiny resident set, touched on a fixed cadence. Its latency percentiles are
# what a user would feel.
cat > "$OUT/canary.py" <<'PY'
import time, sys, random
mb = 64
pages = [bytearray(1024*1024) for _ in range(mb)]
lat = []
end = time.time() + float(sys.argv[1])
while time.time() < end:
    t0 = time.perf_counter()
    for p in pages:
        p[0] = (p[0] + 1) % 256
    lat.append((time.perf_counter() - t0) * 1000)
    time.sleep(0.5)
lat.sort()
def pct(p): return lat[min(len(lat)-1, int(len(lat)*p))]
print(f"canary_ms p50={pct(0.50):.2f} p90={pct(0.90):.2f} p99={pct(0.99):.2f} n={len(lat)}")
PY

# ── Sampling ────────────────────────────────────────────────────────────────
sample() {
    local t=$1
    local pswpin pswpout psi_some psi_full
    pswpin=$(awk '/^pswpin /{print $2}' /proc/vmstat)
    pswpout=$(awk '/^pswpout /{print $2}' /proc/vmstat)
    psi_some=$(awk -F'total=' '/^some/{print $2}' /proc/pressure/memory)
    psi_full=$(awk -F'total=' '/^full/{print $2}' /proc/pressure/memory)

    # zram aggregate: orig, compressed, and what zsmalloc actually holds.
    local orig=0 compr=0 used=0
    for d in /sys/block/zram*/mm_stat; do
        [ -r "$d" ] || continue
        read -r o c m _ < "$d"
        orig=$((orig+o)); compr=$((compr+c)); used=$((used+m))
    done

    echo "$t $pswpin $pswpout $psi_some $psi_full $orig $compr $used" >> "$OUT/samples.tsv"
}

echo "output:   $OUT"
echo "limits:   MemoryMax=$MEM_MAX MemorySwapMax=$SWAP_MAX"
echo "load:     $WORKERS workers, ${DURATION}s"
echo

# Per-worker sizes and rates spread across a range rather than uniform.
TOTAL_MB=$(python3 -c "
import re
s='$MEM_MAX'; n=int(re.sub('[^0-9]','',s)); print(int(n*1024*1.5))")
PER=$(( TOTAL_MB / WORKERS ))

echo "t pswpin pswpout psi_some psi_full zram_orig zram_compr zram_used" > "$OUT/samples.tsv"

systemd-run --quiet --scope --collect \
    -p MemoryMax="$MEM_MAX" -p MemorySwapMax="$SWAP_MAX" \
    --unit="swap-bench-$$" \
    bash -c '
        for i in $(seq 1 '"$WORKERS"'); do
            rate=$(python3 -c "print(0.001 * (1 + $i % 4))")
            python3 '"$OUT"'/worker.py $i '"$PER"' $rate &
        done
        wait
    ' &
SCOPE_PID=$!

python3 "$OUT/canary.py" "$DURATION" > "$OUT/canary.txt" &
CANARY_PID=$!

for ((t=0; t<DURATION; t+=SAMPLE)); do
    sample "$t"
    sleep "$SAMPLE"
done

kill "$SCOPE_PID" 2>/dev/null || true
systemctl stop "swap-bench-$$.scope" 2>/dev/null || true
wait "$CANARY_PID" 2>/dev/null || true

# ── Report ──────────────────────────────────────────────────────────────────
echo
cat "$OUT/canary.txt" 2>/dev/null || echo "canary produced no output"
python3 - "$OUT/samples.tsv" <<'PY'
import sys
rows=[l.split() for l in open(sys.argv[1]).read().splitlines()[1:] if l.strip()]
if len(rows) < 2:
    print("not enough samples"); sys.exit()
f,l=rows[0],rows[-1]
dt=int(l[0])-int(f[0]) or 1
print(f"swap_out  {(int(l[2])-int(f[2]))*4096/2**20/dt:.1f} MB/s")
print(f"swap_in   {(int(l[1])-int(f[1]))*4096/2**20/dt:.1f} MB/s")
print(f"psi_some  {(int(l[3])-int(f[3]))/1e6/dt*100:.2f}% of wall")
print(f"psi_full  {(int(l[4])-int(f[4]))/1e6/dt*100:.2f}% of wall")
peak=max(rows,key=lambda r:int(r[6]))
o,c,u=int(peak[5]),int(peak[6]),int(peak[7])
if c:
    print(f"ratio     {o/c:.2f}x at peak")
    print(f"zsmalloc  {(u-c)/c*100:.1f}% over compressed size (fragmentation)")
PY
echo
echo "samples: $OUT/samples.tsv"
