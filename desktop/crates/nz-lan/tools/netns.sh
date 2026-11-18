#!/bin/sh
# LN3's two-node harness: two of this crate's test processes inside ONE user
# network namespace, multicasting on a dummy interface — no sudo, no libvirt, and
# nothing on the person's own network (docs/agents/lan-nodes.md, *Two nodes on one
# machine*).
#
# Usage:
#   tools/netns.sh [--runs N] [--binary PATH]
#
#   --runs N       announce → browse → rename → goodbye, N times (default 20)
#   --binary PATH  the test binary to run. `cargo test --test discovery` hands its
#                  own path over; without it this script builds the test itself.
#
# The LAN interface is a `dummy` device, or `lo` as a last resort (and then the IPv6
# scope assertion is skipped with a printed reason). NZ_LAN_NETNS_DEVICE=auto|lo pins
# it, for diagnosing a machine or a namespace where the `dummy` module is out of
# reach.
#
# The kernel requirement is unprivileged user namespaces (`unshare -rn`). Arch
# allows them. Ubuntu 24.04 restricts them through AppArmor, and the dedicated step
# of .github/workflows/desktop.yaml lifts that first:
#     sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0
#
# When `unshare -rn true` fails this script SKIPS with a printed reason and exits
# 0 — unless NZ_LAN_REQUIRE_NETNS=1, which turns the same situation into a failure
# (the CI step sets it: a job that silently stopped testing discovery is worse than
# no job at all).
#
# Nothing goes to /tmp (a tmpfs, i.e. RAM): the FIFO, the logs and the report live
# under the workspace's target/ directory. Only the PIDs this script started are
# ever killed.
#
# Every line the Rust roles print starts with `NZLAN `; the test that calls this
# script parses those lines and asserts the note's numbers.

set -eu

runs=20
binary=${NZ_LAN_NETNS_BIN:-}

while [ $# -gt 0 ]; do
    case "$1" in
        --runs)
            [ $# -ge 2 ] || { echo "netns.sh: --runs needs a value" >&2; exit 2; }
            runs=$2
            shift 2
            ;;
        --binary)
            [ $# -ge 2 ] || { echo "netns.sh: --binary needs a value" >&2; exit 2; }
            binary=$2
            shift 2
            ;;
        -h|--help)
            sed -n '2,32p' "$0"
            exit 0
            ;;
        *)
            echo "netns.sh: unknown argument: $1" >&2
            exit 2
            ;;
    esac
done

case "$runs" in
    ''|*[!0-9]*) echo "netns.sh: --runs must be a positive integer, got '$runs'" >&2; exit 2 ;;
esac
[ "$runs" -ge 1 ] || { echo "netns.sh: --runs must be at least 1" >&2; exit 2; }

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
target_dir=${CARGO_TARGET_DIR:-$(CDPATH= cd -- "$here/../../.." && pwd)/target}

require=${NZ_LAN_REQUIRE_NETNS:-0}

for tool in unshare ip; do
    command -v "$tool" >/dev/null 2>&1 || {
        echo "netns.sh: '$tool' is missing (util-linux / iproute2)" >&2
        exit 1
    }
done

if ! unshare -rn true 2>/dev/null; then
    reason="user namespaces are not available here: 'unshare -rn true' failed (Ubuntu 24.04 restricts them through AppArmor)"
    if [ "$require" = "1" ]; then
        echo "netns.sh: $reason" >&2
        exit 1
    fi
    echo "NZLAN HARNESS SKIP $reason"
    exit 0
fi

if [ -z "$binary" ]; then
    # No test binary was handed over: build the role binary ourselves. The JSON
    # message carries the path of the executable `cargo test --no-run` produced.
    binary=$(cd "$here/.." && cargo test --test discovery --no-run --message-format=json 2>/dev/null \
        | grep -o '"executable":"[^"]*"' | tail -n 1 | cut -d'"' -f4)
fi

if [ -z "$binary" ] || [ ! -x "$binary" ]; then
    echo "netns.sh: no runnable test binary (build it with 'cargo test -p nz-lan --test discovery --no-run')" >&2
    exit 1
fi

work="$target_dir/netns-ln3"
mkdir -p "$work"

# The namespace: `lo` for the two processes' control sockets, and ONE interface for
# the LAN — a machine with exactly one network interface, which is what the app
# announces on. It must carry IPv6 multicast, and `lo` does not: Linux delivers an
# IPv4 multicast packet to a socket joined on `lo` and drops the IPv6 one (measured:
# `/proc/net/snmp6` counts 3 out and 0 in for the round trip that works over IPv4),
# so the link-local address whose scope the test asserts (RFC 4007) would never
# arrive. A `dummy` device loops both families (a `veth` end does not: measured, the
# same announcement carries 10.42.0.1 only). Where the `dummy` module cannot be
# loaded from a user namespace, the run falls back to `lo` — IPv4 discovery still
# measured, IPv6 scope skipped with a printed reason.
NZ_LAN_HARNESS_BINARY=$binary \
NZ_LAN_HARNESS_RUNS=$runs \
NZ_LAN_HARNESS_WORK=$work \
NZ_LAN_HARNESS_REQUIRE=$require \
NZ_LAN_HARNESS_DEVICE=${NZ_LAN_NETNS_DEVICE:-auto} \
    unshare -rn sh -s <<'INNER'
set -eu

binary=$NZ_LAN_HARNESS_BINARY
runs=$NZ_LAN_HARNESS_RUNS
work=$NZ_LAN_HARNESS_WORK
require=$NZ_LAN_HARNESS_REQUIRE
device=$NZ_LAN_HARNESS_DEVICE

announce_port=45940
control_port=45941
browse_port=45942
witness_port=45943
drop_port=45944

ip link set lo up

lan_if=
if [ "$device" != "lo" ] && ip link add nz0 type dummy 2>/dev/null; then
    lan_if=nz0
fi

if [ -n "$lan_if" ]; then
    ip link set "$lan_if" up
    ip addr add 10.42.0.1/24 dev "$lan_if"
    ip route add 224.0.0.0/4 dev "$lan_if"
else
    lan_if=lo
    ip link set lo multicast on
    ip route add 224.0.0.0/4 dev lo
fi

# `lo` is the crate's loopback mode; a device of its own is the app's mode (every up,
# non-loopback interface).
loopback=0
if [ "$lan_if" = "lo" ]; then
    loopback=1
fi

ipv6=0
if [ "$lan_if" != "lo" ] && ip -6 addr add fe80::1/64 dev "$lan_if" 2>/dev/null; then
    ip -6 route add ff00::/8 dev "$lan_if" 2>/dev/null || true
    ipv6=1
fi
if [ "$ipv6" = "0" ]; then
    if [ "$lan_if" != "lo" ] && [ "$require" = "1" ]; then
        echo "netns.sh: could not add a link-local IPv6 address to $lan_if, so the scope assertion cannot run" >&2
        exit 1
    fi
    echo "NZLAN HARNESS NOTE no IPv6 on the LAN interface ($lan_if): the link-local scope assertion is skipped"
fi
echo "NZLAN HARNESS DEVICE $lan_if loopback=$loopback ipv6=$ipv6"

run=1
while [ "$run" -le "$runs" ]; do
    fifo=$work/ready.$run.fifo
    announce_log=$work/announce.$run.log
    browse_log=$work/browse.$run.log
    rm -f "$fifo"
    mkfifo "$fifo"

    NZ_LAN_NETNS_ROLE=announce \
    NZ_LAN_NETNS_ANNOUNCE_PORT=$announce_port \
    NZ_LAN_NETNS_CONTROL_PORT=$control_port \
    NZ_LAN_NETNS_WITNESS_PORT=$witness_port \
    NZ_LAN_NETNS_DROP_PORT=$drop_port \
    NZ_LAN_NETNS_LOOPBACK=$loopback \
        "$binary" --exact netns_announcer --nocapture >"$fifo" 2>"$announce_log" &
    announcer=$!
    # Only this script's own child, and only if it outlives a failed run.
    trap 'kill "$announcer" 2>/dev/null || true' EXIT INT TERM

    # READY is an event (the announcement was resolved), never a sleep: the
    # announcer's stdout is a FIFO and the read below blocks until it writes.
    exec 3<"$fifo"
    ready=
    while IFS= read -r line <&3; do
        case "$line" in
            "NZLAN READY"*) ready=$line; break ;;
        esac
    done
    if [ -n "$ready" ]; then
        echo "$ready"
    fi
    if [ -z "$ready" ]; then
        echo "netns.sh: the announcer never announced (run $run)" >&2
        sed 's/^/  | /' "$announce_log" >&2
        exit 1
    fi

    set +e
    NZ_LAN_NETNS_ROLE=browse \
    NZ_LAN_NETNS_ANNOUNCE_PORT=$announce_port \
    NZ_LAN_NETNS_CONTROL_PORT=$control_port \
    NZ_LAN_NETNS_BROWSE_PORT=$browse_port \
    NZ_LAN_NETNS_LOOPBACK=$loopback \
    NZ_LAN_NETNS_IPV6=$ipv6 \
        "$binary" --exact netns_browser --nocapture >"$browse_log" 2>&1
    status=$?
    set -e

    grep '^NZLAN ' "$browse_log" || true
    if [ "$status" -ne 0 ]; then
        echo "netns.sh: run $run failed (the browser exited with $status)" >&2
        sed 's/^/  | /' "$browse_log" >&2
        sed 's/^/  | /' "$announce_log" >&2
        exit 1
    fi

    # The announcer stops on the browser's `stop` and prints the rest: read it out
    # and let it leave on its own, so a leaked thread fails the run here.
    threads_ok=0
    while IFS= read -r line <&3; do
        case "$line" in
            NZLAN*) echo "$line" ;;
        esac
        case "$line" in
            "NZLAN THREADS_OK"*) threads_ok=1 ;;
        esac
    done
    exec 3<&-

    if ! wait "$announcer"; then
        echo "netns.sh: the announcer failed (run $run)" >&2
        sed 's/^/  | /' "$announce_log" >&2
        exit 1
    fi
    if [ "$threads_ok" != "1" ]; then
        echo "netns.sh: the announcer did not prove its threads were joined (run $run)" >&2
        sed 's/^/  | /' "$announce_log" >&2
        exit 1
    fi

    rm -f "$fifo"
    run=$((run + 1))
done

echo "NZLAN HARNESS OK runs=$runs ipv6=$ipv6"
INNER
