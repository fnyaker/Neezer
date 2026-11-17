#!/usr/bin/env bash
# mobile/tools/perf-server.sh — the synthetic Neezer server the phone measures
# against (docs/agents/mobile.md, MQ1: "starts tools/perf_api.py --serve on a
# free port, prints the URL the emulator uses: http://10.0.2.2:<port>").
#
#   mobile/tools/perf-server.sh start            # 8 000 tracks, a 4 000-track playlist
#   mobile/tools/perf-server.sh status
#   mobile/tools/perf-server.sh stop
#
# Why this and not `python tools/perf_api.py --serve &`:
#   - the emulator reaches the host loopback as 10.0.2.2, and a lot that forgets
#     it spends an hour on "connection refused" (the guest has no 127.0.0.1);
#   - perf_api builds its library in $TMPDIR/nsperf-*, and /tmp on this machine
#     is a RAM tmpfs (2.3 GB at 20 000 tracks filled it once, for everybody), so
#     TMPDIR is moved onto disk under ~/.cache;
#   - it is stopped with SIGTERM, which is the signal perf_api removes its
#     library on. A SIGKILL leaves the directory for the next launch to sweep.
#
# The account every bench logs in with is tools/perf_api.py's own published
# fixture: bench / Bench1 (the password is NOT printed by this script; the JSON
# sidecar names it because the next lot's bench has to type it).
#
# State: ~/.cache/neezer-perf/<port>.{pid,log,json} and tmp/.

set -euo pipefail

repo_root() {
    # tools/ lives at mobile/tools, so the repository is two levels up from here
    # and absolute, whatever the caller's cwd.
    local here
    here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    (cd "$here/../.." && pwd)
}

readonly repo="$(repo_root)"
readonly state_dir="${NEEZER_PERF_STATE:-$HOME/.cache/neezer-perf}"
readonly py="${PYTHON:-$repo/.venv/bin/python}"

die() {
    printf 'perf-server.sh: %s\n' "$*" >&2
    exit 1
}
note() { printf 'perf-server.sh: %s\n' "$*" >&2; }

usage() {
    cat <<'EOF'
Usage: mobile/tools/perf-server.sh <start|stop|status> [options]

  start   Start tools/perf_api.py --serve (8 000 tracks, a 4 000-track playlist,
          Deezer off) and print the two URLs, once the server answers:
            host=http://127.0.0.1:<port>
            emulator=http://10.0.2.2:<port>
          Options after `start` are passed to perf_api.py verbatim
          (e.g. --tracks 20000, --archived, --covers cover.jpg, --port 5799).

  stop    SIGTERM (perf_api deletes its library on it), then SIGKILL after 15 s.
  status  The port, the PID, liveness and the login the library was seeded with.

Environment:
  PYTHON              the interpreter (default <repo>/.venv/bin/python, else python3)
  NEEZER_PERF_STATE   state directory (default ~/.cache/neezer-perf)
EOF
}

command="${1:-}"
[[ $# -gt 0 ]] && shift

python_bin() {
    if [[ -x "$py" ]]; then
        printf '%s\n' "$py"
    elif command -v python3 >/dev/null 2>&1; then
        command -v python3
    else
        die "no Python: set PYTHON, or install the repository's .venv"
    fi
}

# tools/perf_api.py imports peewee; a bare python3 without the project installed
# fails with a traceback that says nothing about the real cause.
require_python_deps() {
    local p
    p="$(python_bin)"
    "$p" - <<'PY' || die "the interpreter missing peewee: pip install -e . (or set PYTHON to the repository's .venv)"
import sys
try:
    import peewee, flask  # noqa: F401
except ModuleNotFoundError as e:
    print(f"missing: {e.name}", file=sys.stderr)
    sys.exit(1)
PY
}

port_of_file() {
    # The port is IN the file name: <port>.pid.
    local f="$1"
    basename "$f" .pid
}

# The newest live server, or nothing. One at a time is the rule: a second server
# means a second 900 MB library on a machine another lot is measuring on.
running_ports() {
    local f pid
    for f in "$state_dir"/*.pid; do
        [[ -f "$f" ]] || continue
        pid="$(tr -dc '0-9' <"$f")"
        [[ -n "$pid" ]] || continue
        if [[ -r "/proc/$pid/cmdline" ]] && tr '\0' ' ' <"/proc/$pid/cmdline" | grep -q "perf_api.py"; then
            printf '%s\n' "$(port_of_file "$f")"
        fi
    done
}

cmd_start() {
    require_python_deps
    mkdir -p "$state_dir/tmp"

    local existing
    existing="$(running_ports | head -1)"
    [[ -z "$existing" ]] || die "perf_api.py is already serving on port $existing (stop it first: two libraries are 1.8 GB and two servers compete with the measure)"

    # perf_api picks a free port when --port is absent (0.0.0.0 by default), and
    # prints `READY <port> <json>` once it serves. Reading that line is the
    # event; no sleep and no "is it up yet" guess.
    (
        cd "$repo"
        export TMPDIR="$state_dir/tmp"
        exec "$(python_bin)" tools/perf_api.py --runs 1 --serve --tracks 8000 --big-playlist 4000 "$@"
    ) >"$state_dir/last.log" 2>&1 &
    local pid=$!
    printf '%s\n' "$pid" >"$state_dir/starting.pid"

    local started=$SECONDS port="" ready=""
    while (( SECONDS - started < 180 )); do
        if ! kill -0 "$pid" 2>/dev/null; then
            rm -f "$state_dir/starting.pid"
            note "perf_api.py exited while seeding; last log lines ($state_dir/last.log):"
            tail -n 20 "$state_dir/last.log" >&2 || true
            die "the bench server did not start"
        fi
        ready="$(grep -m1 -E '^READY [0-9]+ ' "$state_dir/last.log" 2>/dev/null || true)"
        [[ -n "$ready" ]] && break
        sleep 0.2
    done
    [[ -n "$ready" ]] || {
        kill -TERM "$pid" 2>/dev/null || true
        die "the bench server did not print READY within 180 s (see $state_dir/last.log)"
    }

    port="$(awk '{print $2}' <<<"$ready")"
    local ids
    ids="$(cut -d' ' -f3- <<<"$ready")"

    mv "$state_dir/starting.pid" "$state_dir/$port.pid"
    printf '%s\n' "$ids" >"$state_dir/$port.json"

    # 10.0.2.2 is the HOST's loopback as the emulator's NAT sees it: the guest
    # has no 127.0.0.1 of ours, and `localhost` there is the phone itself.
    printf 'host=http://127.0.0.1:%s\n' "$port"
    printf 'emulator=http://10.0.2.2:%s\n' "$port"
    printf 'port=%s pid=%s login=bench/Bench1 ids=%s\n' "$port" "$pid" "$state_dir/$port.json"
}

cmd_stop() {
    local ports
    ports="$(running_ports)"
    if [[ -z "$ports" ]]; then
        note "no bench server running"
        return 0
    fi
    local port pid
    for port in $ports; do
        pid="$(tr -dc '0-9' <"$state_dir/$port.pid")"
        # SIGTERM first: perf_api deletes its library on it, and /tmp is RAM.
        kill -TERM "$pid" 2>/dev/null || true
        local waited=0
        while (( waited < 150 )) && kill -0 "$pid" 2>/dev/null; do
            sleep 0.1
            waited=$((waited + 1))
        done
        if kill -0 "$pid" 2>/dev/null; then
            note "pid $pid ignored SIGTERM for 15 s: SIGKILL (its library is swept by the next launch)"
            kill -KILL "$pid" 2>/dev/null || true
            sleep 0.2
        fi
        rm -f "$state_dir/$port.pid"
        note "stopped the bench server on port $port (pid $pid)"
        printf 'port=%s stopped=1\n' "$port"
    done
}

cmd_status() {
    local ports
    ports="$(running_ports)"
    if [[ -z "$ports" ]]; then
        printf 'running=0\n'
        return 0
    fi
    local port
    for port in $ports; do
        printf 'running=1 port=%s pid=%s host=http://127.0.0.1:%s emulator=http://10.0.2.2:%s ids=%s\n' \
            "$port" "$(tr -dc '0-9' <"$state_dir/$port.pid")" "$port" "$port" "$state_dir/$port.json"
    done
}

case "$command" in
    start) cmd_start "$@" ;;
    stop) cmd_stop ;;
    status) cmd_status ;;
    "" | -h | --help | help) usage ;;
    *) die "unknown command: $command (try --help)" ;;
esac
