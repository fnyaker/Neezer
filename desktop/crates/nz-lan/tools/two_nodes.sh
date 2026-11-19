#!/bin/sh
# LN15's two-node journey harness: two `lan_node` PROCESSES inside ONE user
# network namespace, on a dummy interface, with real mDNS — no sudo, nothing on
# the person's own network (docs/agents/lan-nodes.md, *Two nodes on one machine*,
# rung 2, and the LN15 lot).
#
# Usage:
#   tools/two_nodes.sh [--runs N] [--tracks N] [--binary PATH]
#                      [--throughput] [--big-mb N] [--keep]
#
#   --runs N       journey runs: announce → discover → authenticate → fetch → stop
#                  (default 20, the count the note's Discovery latency row asks for)
#   --tracks N     complete downloads the PC shares (default 2 000)
#   --binary PATH  the `lan_node` example to run; without it this script builds it
#   --throughput   one 64 MiB fetch, reported in MB/s (--big-mb N changes the size)
#   --keep         keep the data directories of the run (they are reused anyway)
#
# The namespace has `lo` (nothing but the two processes' own loopback) and ONE
# `dummy` interface for the LAN, which is also the app's real discovery policy
# (every up, non-loopback interface). There is NO route out: the node's API base
# is 127.0.0.1:9, so the server is unreachable for the whole run — that is the trip
# the lot exists for.
#
# Where the `dummy` module cannot be loaded from a user namespace the script
# FALLS BACK to `lo` with multicast and says so (`lo` carries IPv4 multicast; LN3
# measured that it drops the IPv6 one, which this harness does not assert). A
# machine without unprivileged user namespaces SKIPS with a printed reason and
# exits 0 — unless NZ_LAN_REQUIRE_NETNS=1, which turns the same situation into a
# failure (a CI step that silently stopped testing the journey is worse than no
# step at all).
#
# Nothing goes to /tmp (a tmpfs, i.e. RAM): the data directories, the FIFOs and
# the logs live under the workspace's `target/two-nodes-ln15/`, and the media
# fixtures are made there by ffmpeg. Only the PIDs this script started are killed,
# by PID.
#
# Every line the nodes print starts with `NZLAN `; this script prints its own
# report as `NZLAN …` too, so a reader (or a test) sees one stream.

set -eu

runs=20
tracks=2000
binary=${NZ_LAN_TWO_NODES_BIN:-}
throughput=0
big_mb=64
keep=0

while [ $# -gt 0 ]; do
    case "$1" in
        --runs)
            [ $# -ge 2 ] || { echo "two_nodes.sh: --runs needs a value" >&2; exit 2; }
            runs=$2
            shift 2
            ;;
        --tracks)
            [ $# -ge 2 ] || { echo "two_nodes.sh: --tracks needs a value" >&2; exit 2; }
            tracks=$2
            shift 2
            ;;
        --binary)
            [ $# -ge 2 ] || { echo "two_nodes.sh: --binary needs a value" >&2; exit 2; }
            binary=$2
            shift 2
            ;;
        --throughput)
            throughput=1
            shift
            ;;
        --big-mb)
            [ $# -ge 2 ] || { echo "two_nodes.sh: --big-mb needs a value" >&2; exit 2; }
            big_mb=$2
            shift 2
            ;;
        --keep)
            keep=1
            shift
            ;;
        -h|--help)
            sed -n '2,45p' "$0"
            exit 0
            ;;
        *)
            echo "two_nodes.sh: unknown argument: $1" >&2
            exit 2
            ;;
    esac
done

case "$runs" in
    ''|*[!0-9]*) echo "two_nodes.sh: --runs must be a positive integer, got '$runs'" >&2; exit 2 ;;
esac
case "$tracks" in
    ''|*[!0-9]*) echo "two_nodes.sh: --tracks must be a positive integer, got '$tracks'" >&2; exit 2 ;;
esac
[ "$runs" -ge 1 ] || { echo "two_nodes.sh: --runs must be at least 1" >&2; exit 2; }
[ "$tracks" -ge 1 ] || { echo "two_nodes.sh: --tracks must be at least 1" >&2; exit 2; }

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
target_dir=${CARGO_TARGET_DIR:-$(CDPATH= cd -- "$here/../../.." && pwd)/target}
require=${NZ_LAN_REQUIRE_NETNS:-0}

for tool in unshare ip ffmpeg; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "two_nodes.sh: '$tool' is missing (util-linux / iproute2 / ffmpeg)" >&2
        exit 1
    }
done

if ! unshare -rn true 2>/dev/null; then
    reason="user namespaces are not available here: 'unshare -rn true' failed (Ubuntu 24.04 restricts them through AppArmor)"
    if [ "$require" = "1" ]; then
        echo "two_nodes.sh: $reason" >&2
        exit 1
    fi
    echo "NZLAN HARNESS SKIP $reason"
    exit 0
fi

if [ -z "$binary" ]; then
    ( cd "$here/.." && cargo build -p nz-lan --example lan_node --message-format=json 2>/dev/null \
        | grep -o '"executable":"[^"]*"' | tail -n 1 | cut -d'"' -f4 ) > "$target_dir/.ln15-binary.$$" 2>/dev/null || true
    built=$(cat "$target_dir/.ln15-binary.$$" 2>/dev/null || true)
    rm -f "$target_dir/.ln15-binary.$$"
    if [ -z "$built" ] || [ ! -x "$built" ]; then
        echo "two_nodes.sh: could not build the lan_node example (run 'cargo build -p nz-lan --example lan_node')" >&2
        exit 1
    fi
    binary=$built
fi
[ -x "$binary" ] || { echo "two_nodes.sh: '$binary' is not executable" >&2; exit 1; }

work="$target_dir/two-nodes-ln15"
mkdir -p "$work"

NZ_LAN_TWO_NODES_BINARY=$binary \
NZ_LAN_TWO_NODES_WORK=$work \
NZ_LAN_TWO_NODES_RUNS=$runs \
NZ_LAN_TWO_NODES_TRACKS=$tracks \
NZ_LAN_TWO_NODES_THROUGHPUT=$throughput \
NZ_LAN_TWO_NODES_BIG_MB=$big_mb \
NZ_LAN_TWO_NODES_REQUIRE=$require \
    unshare -rn sh -s <<'INNER'
set -eu

binary=$NZ_LAN_TWO_NODES_BINARY
work=$NZ_LAN_TWO_NODES_WORK
runs=$NZ_LAN_TWO_NODES_RUNS
tracks=$NZ_LAN_TWO_NODES_TRACKS
throughput=$NZ_LAN_TWO_NODES_THROUGHPUT
big_mb=$NZ_LAN_TWO_NODES_BIG_MB
require=$NZ_LAN_TWO_NODES_REQUIRE

pc_port=46060
phone_port=46061

ip link set lo up

lan_if=
if ip link add nz0 type dummy 2>/dev/null; then
    lan_if=nz0
    ip link set nz0 up
    ip addr add 10.42.0.1/24 dev nz0
    ip route add 224.0.0.0/4 dev nz0
fi
if [ -z "$lan_if" ]; then
    # `lo` still carries IPv4 multicast: the journey runs, only the IPv6 scope
    # story (LN3's) is out of reach here, which this harness never asserts.
    lan_if=lo
    ip link set lo multicast on
    ip route add 224.0.0.0/4 dev lo
    echo "NZLAN HARNESS NOTE no dummy interface: discovery on lo (IPv4 only)"
fi
echo "NZLAN HARNESS DEVICE $lan_if"

# A 0.1 s tone: a real FLAC (the decoder probe of a fetch refuses anything else),
# small enough that 2 000 copies are 30 MB under the worktree's target/.
tone=$work/tone.flac
if [ ! -f "$tone" ]; then
    ffmpeg -hide_banner -loglevel error \
        -f lavfi -i "aevalsrc=0.5*cos(2*PI*440*t):s=48000:d=0.1:c=stereo" \
        -c:a flac "$tone"
fi
tone_bytes=$(wc -c < "$tone" | tr -d ' ')
tone_sha=$(sha256sum "$tone" | cut -d' ' -f1)

# The PC is prepared ONCE (its key, its LAN trust and its 2 000 downloads), then
# reused by every run: a run must measure discovery and the transfer, not a
# library import.
if [ ! -f "$work/a/harness-trust" ]; then
    "$binary" --data-dir "$work/a" --init-trust --name PC --port "$pc_port" \
        --discovery mdns --seed-file "$tone" --seed-copies "$tracks" \
        --deadline-ms 20000 --wait-ms 600 >"$work/prepare.log" 2>&1 || {
            echo "two_nodes.sh: could not prepare the PC node" >&2
            sed 's/^/  | /' "$work/prepare.log" >&2
            exit 1
        }
    grep '^NZLAN ' "$work/prepare.log" || true
fi

# The phone is a fresh, EMPTY device of the same account: the trip's phone.
if [ ! -f "$work/b/lan.sqlite" ]; then
    : # created by its first run
fi

big=$work/big.flac
if [ "$throughput" = "1" ] && [ ! -f "$big" ]; then
    seconds=$((big_mb * 3))
    ffmpeg -hide_banner -loglevel error \
        -f lavfi -i "anoisesrc=d=$seconds:c=pink:r=96000:a=0.6" -ac 2 -c:a flac "$big"
fi
big_bytes=0
if [ -f "$big" ]; then
    big_bytes=$(wc -c < "$big" | tr -d ' ')
fi

median() { sort -n | awk '{v[NR]=$1} END {if (NR==0) {print 0; exit} i=int((NR+1)/2); print v[i]}'; }
p95() { sort -n | awk '{v[NR]=$1} END {if (NR==0) {print 0; exit} i=int(NR*0.95); if (i<1) i=1; if (i>NR) i=NR; print v[i]}'; }

run=1
# The journey runs, plus ONE last run with the phone's store kept: the note's
# "last-known address after a network change ≤ 300 ms" row, on the same pair.
last_run=$((runs + 1))
while [ "$run" -le "$last_run" ]; do
    fifo=$work/ready.$run.fifo
    pc_log=$work/pc.$run.log
    phone_log=$work/phone.$run.log
    out=$work/fetched.$run.flac
    rm -f "$fifo" "$out"
    mkfifo "$fifo"

    # Each journey run is a NEW phone on the network: `lan.sqlite` holds the peers'
    # last addresses, and keeping it would measure the reconnect path instead of
    # mDNS. The example rewrites its trust rows at every start, so the store is
    # cheap to drop (its device key is a file, and stays).
    if [ "$run" -le "$runs" ]; then
        rm -f "$work/b/lan.sqlite" "$work/b/lan.sqlite-wal" "$work/b/lan.sqlite-shm"
    fi

    # The PC announces on the LAN and waits; its READY line is the event the
    # phone's run starts on (the announcement is resolvable, RFC 6762's probing
    # included). It ends its OWN run 2.5 s later — long enough for the phone's fetch
    # and for quinn's counters to be sampled once while a peer exists, which is what
    # makes the "first contact" byte count a measured number (LN11's lesson: a
    # counter read once a second is not an event).
    "$binary" --data-dir "$work/a" --trust-dir "$work/a" --name PC --port "$pc_port" \
        --discovery mdns --deadline-ms 20000 --wait-ms 2500 >"$fifo" 2>"$work/pc.$run.err" &
    pc_pid=$!
    trap 'kill "$pc_pid" 2>/dev/null || true' EXIT INT TERM

    exec 3<"$fifo"
    ready=
    while IFS= read -r line <&3; do
        printf '%s\n' "$line" >>"$pc_log"
        case "$line" in
            "NZLAN READY"*) ready=$line; break ;;
        esac
    done
    if [ -z "$ready" ]; then
        echo "two_nodes.sh: the PC never became ready (run $run)" >&2
        sed 's/^/  | /' "$work/pc.$run.err" >&2
        exit 1
    fi
    # Drain the rest so the PC never writes into a closed FIFO.
    cat <&3 >>"$pc_log" &
    drain=$!
    exec 3<&-

    set +e
    timeout 60 "$binary" --data-dir "$work/b" --trust-dir "$work/a" --name Téléphone \
        --port "$phone_port" --discovery mdns --fetch "70000001=$out" \
        --deadline-ms 30000 >"$phone_log" 2>&1
    status=$?
    set -e

    grep '^NZLAN ' "$phone_log" || true
    if [ "$status" -ne 0 ]; then
        echo "two_nodes.sh: the phone's run failed (exit $status, run $run)" >&2
        sed 's/^/  | /' "$phone_log" >&2
        sed 's/^/  | /' "$work/pc.$run.err" >&2
        exit 1
    fi

    # The PC ends on its own (2.5 s) and prints its own counters: they are the
    # first contact's bytes, from the side that sent them.
    wait "$pc_pid" 2>/dev/null || true
    wait "$drain" 2>/dev/null || true
    trap - EXIT INT TERM
    rm -f "$fifo"
    pc_wire=$(grep -o 'NZLAN WIRE out=[0-9]* out_bytes=[0-9]* in=[0-9]* in_bytes=[0-9]*' "$pc_log" | tail -n 1)

    decided=$(grep -o 'NZLAN DECIDED 70000001 [^ ]*' "$phone_log" | awk '{print $4}')
    case "$decided" in
        peer:*) ;;
        *)
            echo "two_nodes.sh: the phone did not decide on the PC ($decided, run $run)" >&2
            exit 1
            ;;
    esac
    if [ "$(sha256sum "$out" | cut -d' ' -f1)" != "$tone_sha" ]; then
        echo "two_nodes.sh: the fetched file is not the PC's (run $run)" >&2
        exit 1
    fi
    got_bytes=$(wc -c < "$out" | tr -d ' ')
    if [ "$got_bytes" != "$tone_bytes" ]; then
        echo "two_nodes.sh: fetched $got_bytes bytes, the PC's file is $tone_bytes (run $run)" >&2
        exit 1
    fi

    ready_ms=$(grep -o 'NZLAN READY port=[0-9]* ms=[0-9]*' "$phone_log" | awk -F= '{print $3}')
    peer_ms=$(grep -o 'NZLAN PEER ms=[0-9]*' "$phone_log" | awk -F= '{print $2}')
    fetch_ms=$(grep -o 'NZLAN FETCHED 70000001 bytes=[0-9]* quality=[^ ]* sha256=[0-9a-f]* ms=[0-9]*' "$phone_log" | awk -F= '{print $NF}')
    memory=$(grep -o 'NZLAN MEMORY [0-9]*' "$phone_log" | awk '{print $3}')
    wire=$(grep -o 'NZLAN WIRE .*' "$phone_log")
    if [ -z "$ready_ms" ] || [ -z "$peer_ms" ]; then
        echo "two_nodes.sh: the phone's measures are missing (run $run)" >&2
        exit 1
    fi
    discovery=$((peer_ms - ready_ms))
    if [ "$run" -le "$runs" ]; then
        printf '%s\n' "$discovery" >>"$work/discovery.txt"
        printf 'NZLAN RUN %s discovery_ms=%s start_ms=%s peer_ms=%s fetch_ms=%s bytes=%s memory=%s %s PC %s\n' \
            "$run" "$discovery" "$ready_ms" "$peer_ms" "$fetch_ms" "$got_bytes" "$memory" "$wire" "$pc_wire"
    else
        printf 'NZLAN LAST-KNOWN ms=%s peer_ms=%s %s\n' "$discovery" "$peer_ms" "$wire"
    fi

    run=$((run + 1))
done

p50=$(median <"$work/discovery.txt")
p95v=$(p95 <"$work/discovery.txt")
max=$(sort -n "$work/discovery.txt" | tail -n 1)
echo "NZLAN JOURNEY OK runs=$runs tracks=$tracks discovery_p50_ms=$p50 discovery_p95_ms=$p95v discovery_max_ms=$max"

if [ "$throughput" = "1" ]; then
    out=$work/big-fetched.flac
    rm -f "$out"
    fifo=$work/big.fifo
    rm -f "$fifo"
    mkfifo "$fifo"
    # One node holds the big file: seed it as its own track, announce, and let the
    # phone pull it.
    big_track=79999999
    "$binary" --data-dir "$work/a" --trust-dir "$work/a" --name PC --port "$pc_port" \
        --discovery mdns --seed-track "$big_track=$big" --deadline-ms 20000 --wait-ms 60000 \
        >"$fifo" 2>"$work/big-pc.err" &
    pc_pid=$!
    trap 'kill "$pc_pid" 2>/dev/null || true' EXIT INT TERM
    exec 3<"$fifo"
    while IFS= read -r line <&3; do
        case "$line" in
            "NZLAN READY"*) break ;;
        esac
    done
    cat <&3 >"$work/big-pc.log" &
    drain=$!
    exec 3<&-
    timeout 300 "$binary" --data-dir "$work/b" --trust-dir "$work/a" --name Téléphone \
        --port "$phone_port" --discovery mdns --fetch "$big_track=$out" \
        --deadline-ms 60000 >"$work/big-phone.log" 2>&1
    grep '^NZLAN ' "$work/big-phone.log" || true
    kill "$pc_pid" 2>/dev/null || true
    wait "$pc_pid" 2>/dev/null || true
    wait "$drain" 2>/dev/null || true
    trap - EXIT INT TERM
    rm -f "$fifo"
    ms=$(grep -o 'ms=[0-9]*' "$work/big-phone.log" | tail -n 1 | cut -d= -f2)
    if [ -z "$ms" ] || [ "$ms" = "0" ]; then
        echo "two_nodes.sh: the throughput run produced no timing" >&2
        exit 1
    fi
    mbps=$(awk -v b="$big_bytes" -v ms="$ms" 'BEGIN {printf "%.1f", (b / 1048576) / (ms / 1000)}')
    echo "NZLAN THROUGHPUT bytes=$big_bytes ms=$ms mb_per_s=$mbps"
fi

echo "NZLAN HARNESS OK"
INNER
