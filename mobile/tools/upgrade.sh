#!/usr/bin/env bash
# mobile/tools/upgrade.sh — the RELEASE upgrade, IN PLACE: the previous tag's APK
# installed and signed in, then THIS build installed over it, and the session,
# the downloaded title and the versionCode proved to survive (docs/agents/mobile.md,
# "Tests and emulation" → *An upgrade is a first-class path*; plan `plan-mobile-tests`,
# lot T8).
#
#   mobile/tools/upgrade.sh --serial emulator-5580 --from v1.6.7 \
#       --to mobile/android/app/build/outputs/apk/release/app-universal-release.apk
#
# Why it exists. The release page IS the delivery channel: a person downloads
# `neezer-native.apk` and installs it over the build they already have. Android
# only lets that happen when the signer is the SAME certificate and the
# `versionCode` is not lower; and a build whose data migration wipes the session
# or the offline store sends that person back to the setup screen with their
# downloads gone. Every other CI job starts from an EMPTY device, so a rotated
# key, a `versionCode` that went backwards or a destructive migration was green
# everywhere and broken for the only path a person takes.
#
# What it proves, and prints — four facts, none of them "the app launched":
#
#   version_code_up=1   the new APK's `versionCode` is STRICTLY greater than the
#                       installed one (`dumpsys package` after the install, and
#                       `aapt2 dump badging` on both files);
#   signers_equal=1     the two APKs carry the same signer: the certificate's
#                       SHA-256 digest read by `apksigner` on BOTH files, so a
#                       release signed with a fresh key is caught before a phone
#                       has to refuse it;
#   session_kept=1      after the install the app opens on the HOME —
#                       « Accueil Neezer » painted, neither « Adresse du
#                       serveur » nor « Ton compte » anywhere — and a NEW
#                       `GET /api/me` is answered 200, which is the request a
#                       cold `restore()` makes with the cookie stored BEFORE the
#                       upgrade (`/api/me` is not disk-cached; a cover is, so
#                       counting covers would prove nothing here);
#   data_kept=1         the album downloaded before the upgrade still says
#                       « Téléchargé » (the offline store's own answer, drawn by
#                       the `DownloadButton`), and the app's data directory holds
#                       at least as many files and bytes as before it.
#
# A fifth line is the NEGATIVE CONTROL, and it is asserted, not tolerated: the
# DEBUG-signed APK installed over the release one must be refused with
# `INSTALL_FAILED_UPDATE_INCOMPATIBLE` (a different signer). A control that may
# silently pass proves nothing, so its absence is a failure.
#
# Why the sign-in is written here instead of calling `release-signin.sh`
# (T1's recipe). That script owns its server: it starts `perf-server.sh` with a
# throwaway certificate and STOPS it in its `EXIT` trap, and the certificate
# lives in a `mktemp -d` it deletes. The upgrade needs the SAME server, on the
# same host:port, with the SAME certificate, alive on BOTH sides of the install —
# the app pins the leaf it accepted (`nz-api`'s `PinState`), so a new server with
# a new certificate on the same address is refused, by design. The typing steps
# below are `release-signin.sh`'s phase 2, verbatim (`type_into`, `tap_text`,
# `await_text` from `ui.sh`), and the assertions are stricter: the home is
# proved again AFTER the install, with a cover request counted on the server's
# own access log on each side.
#
# Bounded everywhere: `await_text` 10 s (30 s for a cold launch, 180 s for a
# download), no `sleep`, no `uiautomator dump` retried more than its own five
# times, and every `adb` call names the serial it was given. The emulator is NOT
# started here: `ci-emulator.sh` and the `upgrade` job's steps own it through
# `mobile/tools/emu.sh`.
#
# The bench library is small on purpose (`--tracks 200 --big-playlist 3`): the
# measure is "the download survives an install", not a throughput figure, and a
# three-title album downloads in a few seconds over the emulator's loopback.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=mobile/tools/env.sh
. "$here/env.sh"

serial=""
from=""
to=""
debug_apk=""
port="${NEEZER_UPGRADE_PORT:-5690}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --serial) serial="$2"; shift 2 ;;
        --serial=*) serial="${1#*=}"; shift ;;
        --from) from="$2"; shift 2 ;;
        --from=*) from="${1#*=}"; shift ;;
        --to) to="$2"; shift 2 ;;
        --to=*) to="${1#*=}"; shift ;;
        --debug-apk) debug_apk="$2"; shift 2 ;;
        --debug-apk=*) debug_apk="${1#*=}"; shift ;;
        --port) port="$2"; shift 2 ;;
        --port=*) port="${1#*=}"; shift ;;
        -h | --help) sed -n '2,60p' "$0"; exit 0 ;;
        *) printf 'upgrade.sh: unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done

die() {
    printf 'upgrade.sh: %s\n' "$*" >&2
    exit 1
}
note() { printf 'upgrade.sh: %s\n' "$*" >&2; }
step() { printf '\n=== %s\n' "$*"; }

[[ -n "$serial" ]] || die "--serial is required (the device the upgrade runs on)"
[[ -n "$from" ]] || die "--from is required (a release tag, or an APK path)"
[[ -n "$to" ]] || die "--to is required (the APK this build produced)"
[[ -f "$to" ]] || die "no APK at $to"
# The negative control's APK is the DEBUG build: same package, another signer.
# It is derivable from the release path Gradle wrote (…/apk/release/x.apk →
# …/apk/debug/app-universal-debug.apk), so the documented command line above
# needs no fifth argument; the flag exists for a tree that put it elsewhere.
# The universal name comes FIRST because `splits.abi` is on for every variant
# (`app/build.gradle.kts`), so `assembleDebug` writes `app-universal-debug.apk`
# and no `app-debug.apk` at all.
if [[ -z "$debug_apk" ]]; then
    debug_dir="$(cd "$(dirname "$to")/../debug" 2>/dev/null && pwd)"
    for candidate in "$debug_dir/app-universal-debug.apk" "$debug_dir/app-debug.apk"; do
        if [[ -f "$candidate" ]]; then
            debug_apk="$candidate"
            break
        fi
    done
    [[ -n "$debug_apk" ]] || debug_apk="$debug_dir/app-universal-debug.apk"
fi
[[ -f "$debug_apk" ]] || die "no debug APK at $debug_apk (gradle -p mobile/android :app:assembleDebug -PskipRust) — it is the negative control's other signer"

# The accessibility tree, the app's pid, its crash cause and every gesture live
# once, in ui.sh: this script taps the same setup screen, the same album page and
# the same download button that release-signin.sh and release-journey.sh do.
# shellcheck source=mobile/tools/ui.sh
. "$here/ui.sh"

# Nothing big in /tmp (docs/agents/mobile.md, worker rule 8: /tmp is a tmpfs):
# two APKs (≈80 MB together), a certificate, the perf server's library.
root="${NEEZER_UPGRADE_ROOT:-$HOME/.cache/neezer-upgrade}"
mkdir -p "$root"
work="$(mktemp -d "$root/run.XXXXXX")"
perf_state="$work/perf"
logs="$work/logs"
mkdir -p "$perf_state" "$logs"

cleanup() {
    local code=$?
    NEEZER_PERF_STATE="$perf_state" "$here/perf-server.sh" stop >/dev/null 2>&1 || true
    rm -rf "$work"
    exit "$code"
}
trap cleanup EXIT

# --- the two files, and the four facts about them ---------------------------

# The build tools that can read a signature and a badging. 36.0.0 is the version
# mobile.yaml installs; the newest of anything else is the fallback so a machine
# with another SDK still runs.
build_tools() {
    local bt
    for bt in "$(nz_sdk_root)/build-tools/36.0.0" "$(nz_sdk_root)"/build-tools/*; do
        [[ -x "$bt/apksigner" && -x "$bt/aapt2" ]] && { printf '%s\n' "$bt"; return 0; }
    done
    return 1
}

bt="$(build_tools)" || die "no build-tools with both apksigner and aapt2 under $(nz_sdk_root) (36.0.0 is the version mobile.yaml installs)"

apk_badging() { "$bt/aapt2" dump badging "$1"; }
apk_version_code() { apk_badging "$1" | sed -n "s/.*versionCode='\([0-9]*\)'.*/\1/p" | head -1; }
apk_version_name() { apk_badging "$1" | sed -n "s/.*versionName='\([^']*\)'.*/\1/p" | head -1; }
apk_package() { apk_badging "$1" | sed -n "s/^package: name='\([^']*\)'.*/\1/p" | head -1; }
# The certificate's own digest, not the DN: two keys can share a CN, and it is
# the digest Android compares when it decides whether an update is allowed.
apk_signer_sha256() {
    "$bt/apksigner" verify --print-certs "$1" 2>/dev/null |
        sed -n 's/^Signer #1 certificate SHA-256 digest: //p' | head -1
}
apk_signer_dn() {
    "$bt/apksigner" verify --print-certs "$1" 2>/dev/null |
        sed -n 's/^Signer #1 certificate DN: //p' | head -1
}

# `--from` is a release TAG (what a person has) or a path (a tree that kept the
# file). A tag is downloaded from the release page with `gh`, which is the only
# copy anybody has: a locally built "old" APK would not prove the CHANNEL. The
# escalation in the plan says to stop when no asset is downloadable — never to
# substitute one.
resolve_from() {
    local src="$1" dir="$work/from"
    if [[ -f "$src" ]]; then
        printf '%s\n' "$src"
        return 0
    fi
    command -v gh >/dev/null 2>&1 || die "gh is not installed, and --from « $src » is not a file: cannot download a release asset"
    mkdir -p "$dir"
    gh release download "$src" --pattern 'neezer-native.apk' --dir "$dir" >&2 ||
        die "gh could not download neezer-native.apk from release « $src » (asset name or token)"
    [[ -f "$dir/neezer-native.apk" ]] || die "release « $src » carried no neezer-native.apk"
    printf '%s\n' "$dir/neezer-native.apk"
}

step "the two APKs"
previous_apk="$(resolve_from "$from")"
note "previous: $previous_apk ($(stat -c %s "$previous_apk") bytes)"
note "this build: $to ($(stat -c %s "$to") bytes)"

previous_code="$(apk_version_code "$previous_apk")"
current_code="$(apk_version_code "$to")"
current_name="$(apk_version_name "$to")"
previous_signer="$(apk_signer_sha256 "$previous_apk")"
current_signer="$(apk_signer_sha256 "$to")"
previous_dn="$(apk_signer_dn "$previous_apk")"
current_dn="$(apk_signer_dn "$to")"

[[ -n "$previous_code" && -n "$current_code" ]] || die "could not read a versionCode from one of the APKs (aapt2 answered nothing)"
[[ -n "$previous_signer" && -n "$current_signer" ]] || die "could not read a signer certificate from one of the APKs (apksigner answered nothing)"
[[ "$(apk_package "$previous_apk")" == "$(apk_package "$to")" ]] ||
    die "the two APKs are not the same package: $(apk_package "$previous_apk") vs $(apk_package "$to")"

printf 'previous_code=%s current_code=%s current_name=%s\n' "$previous_code" "$current_code" "$current_name"
printf 'previous_signer_dn="%s"\ncurrent_signer_dn="%s"\n' "$previous_dn" "$current_dn"
(( current_code > previous_code )) ||
    die "the new APK's versionCode ($current_code) is not greater than the previous one ($previous_code): Android would refuse the update"
printf 'version_code_up=1 delta=%s\n' "$((current_code - previous_code))"
[[ "$previous_signer" == "$current_signer" ]] ||
    die "the two APKs are signed by different certificates ($previous_signer vs $current_signer): a phone cannot update one with the other"
printf 'signers_equal=1 sha256=%s\n' "$current_signer"

# --- the device, the server, the sign-in ------------------------------------

prepare_device

# The bench server the phone measures against: the SAME process for the whole
# script, with a certificate generated once and kept until the trap. Its port is
# ours (`NEEZER_UPGRADE_PORT`, 5690 by default; 4711 is tone-server.py's and 5610
# is ci-emulator.sh's perf server, so the three never collide).
step "the bench server (one certificate for both sides of the install)"
cert="$work/cert.pem"
key="$work/key.pem"
openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj "/CN=neezer-upgrade" \
    -addext "subjectAltName=IP:10.0.2.2,IP:127.0.0.1" \
    -keyout "$key" -out "$cert" >/dev/null 2>&1
server_out="$(NEEZER_PERF_STATE="$perf_state" "$here/perf-server.sh" start \
    --port "$port" --tracks 200 --playlists 2 --favorites 20 --big-playlist 3 \
    --releases 12 --audio-all --covers "$(nz_repo_root)/tests/assets/cover.jpg" \
    --tls-cert "$cert" --tls-key "$key")"
access_log="$perf_state/last.log"
[[ -f "$access_log" ]] || die "the bench server printed no access log at $access_log: $server_out"
printf 'bench_port=%s\n' "$port"

# THE ALBUM THE BENCH REPORTS, not a hard-coded "album 1": the library's albums
# are ordinary neezer rows keyed by their PROVIDER id (`2000000`, `2000001`, …),
# so `/api/album/1` is no album at all and the page would never paint its
# button. `perf-server.sh` keeps the seed's READY JSON (`… "<album>": "…" …`)
# in `$state_dir/<port>.json` and prints its path as `ids=`, which is the one
# id both sides of the install agree on.
seed_json="$(sed -n 's/.*[[:space:]]ids=\([^[:space:]]*\).*/\1/p' <<<"$server_out" | tail -1)"
[[ -n "$seed_json" && -f "$seed_json" ]] || die "the bench server printed no seed json (ids=…) in: $server_out"
album_id="$(sed -n 's/.*"album":[[:space:]]*"\([^"]*\)".*/\1/p' "$seed_json" | head -1)"
[[ -n "$album_id" ]] || die "the bench server's seed json names no album: $(cat "$seed_json")"
printf 'bench_album=%s\n' "$album_id"

# A cover request, counted on the SERVER: a request exists only once the home's
# cover composable ran and Coil asked the core. This is the fact v1.6.6's crash
# fails. It proves the SIGN-IN (empty cache), NOT the upgrade: `cover_path`
# answers a disk hit with ZERO requests, so after an install the home draws the
# same sleeves from the cache — see `me_now` below for the session's own proof.
covers_now() { grep -cE '"GET /api/cover/[^ ]+ HTTP/[0-9.]+" 200' "$access_log" || true; }

# An AUTHENTICATED request, and the one a cold start always makes: `restore()`
# asks `GET /api/me` with the cookie the device kept (nz-core's session.rs).
# Nothing caches it on disk, so a count that grew AFTER the install can only be
# a session that survived it — a 401 is not counted (the pattern demands 200).
me_now() { grep -cE '"GET /api/me HTTP/[0-9.]+" 200' "$access_log" || true; }

# Installs, signs in, and waits for the home AND its covers. `pm clear` first:
# what a person who just installed the release meets, no session, no pin.
install_and_sign_in() {
    local apk="$1" before covers signed_in=0 started
    adb_dev install -r -g "$apk" 2>&1 | tail -2
    adb_dev shell pm clear "$ui_package" >/dev/null
    # `pm clear` DROPS the installer's `-g` runtime grants, and the first
    # « Télécharger » asks for POST_NOTIFICATIONS (the download draws its progress
    # in the shade): the system dialog — « Allow Neezer (native) to send you
    # notifications? », ALLOW / DON'T ALLOW — covers the album page, and the
    # button never reaches « Téléchargé » (dispatch 45746798927, 180 s). Granting
    # it here restores what `adb install -g` gave and changes nothing the upgrade
    # proves (`release-journey.sh` does the same before the car settings).
    adb_dev shell pm grant "$ui_package" android.permission.POST_NOTIFICATIONS >/dev/null 2>&1 || true
    adb_dev logcat -c -b crash >/dev/null 2>&1 || true
    adb_dev shell am start -W -n "$(nz_launcher_component "$serial")" >/dev/null
    await_text "Adresse du serveur" 30 || fail_screen "the setup screen never painted « Adresse du serveur »"
    type_into 1 "10.0.2.2:$port" || fail_screen "the address field did not take « 10.0.2.2:$port »"
    # OFF: the bench's leaf is self-signed, so it is accepted on screen (trust on
    # first use) — the switch's own label is the LAST node with this text.
    tap_text "Vérifier le certificat" || fail_screen "no « Vérifier le certificat » switch"
    tap_text "Vérifier le serveur" || fail_screen "no « Vérifier le serveur » button"
    await_text "Certificat auto-signé" 30 || fail_screen "no certificate card for the bench server"
    tap_text "Accepter ce certificat" || fail_screen "no « Accepter ce certificat » button"
    await_text "Ton compte" 30 || fail_screen "the account form never opened"
    type_into 1 bench || fail_screen "the account field did not take « bench »"
    # `Bench1`, and `secret` as the THIRD argument: release-signin.sh's call,
    # verbatim. The password is one token (perf_api's `BENCH_PASSWORD`), and a
    # quoted "Bench1 secret" is a two-word string `input text` splits.
    type_into 2 Bench1 secret || fail_screen "the password field did not take the fixture's password"
    tap_text "Se connecter" || fail_screen "no « Se connecter » button"

    # The home, then a NEW cover: both while the process is alive (v1.6.6 died on
    # the first one), and the cover counted at the server.
    before="$(covers_now)"
    started=$SECONDS
    while (( SECONDS - started < 60 )); do
        [[ -n "$(app_pid)" ]] || fail_screen "the app died after signing in"
        if (( signed_in == 0 )) && ui_text | grep -qF "Accueil Neezer"; then
            signed_in=1
        fi
        covers="$(covers_now)"
        if (( signed_in == 1 && covers > before )); then
            printf 'signin=1 covers_served=%s\n' "$covers"
            return 0
        fi
    done
    fail_screen "signed in=${signed_in}, but no new cover was served in 60 s (covers=${covers:-0}, before=$before)"
}

step "the previous release: installed, signed in"
install_and_sign_in "$previous_apk"
check_alive "the previous release's sign-in"
[[ -n "$(app_pid)" ]] || fail_screen "the previous release's process is gone after its sign-in"

# The app's own data directory, read as root. The emulator image is a userdebug
# AOSP build, so `adb root` is allowed; a production phone would refuse, and the
# script says so instead of pretending the data was measured. `du -sk` (KiB) is
# what toybox answers everywhere.
#
# Defined here, before the download, because the failure path reads the SAME
# directory: a download that failed before its first byte is a path or a disk.
ensure_root() {
    local out
    out="$(adb_dev root 2>&1 || true)"
    case "$out" in
        *"cannot run as root"*) return 1 ;;
    esac
    "$(nz_adb)" -s "$serial" wait-for-device >/dev/null 2>&1 || true
    [[ "$(adb_dev shell id -u 2>/dev/null | tr -d '\r')" == "0" ]]
}
data_measure() {
    local dir=/data/data/$ui_package bytes_kib files
    bytes_kib="$(adb_dev shell du -sk "$dir" 2>/dev/null | awk '{print $1}' | tr -d '\r')"
    files="$(adb_dev shell find "$dir" -type f 2>/dev/null | wc -l | tr -d ' \r')"
    printf '%s %s\n' "${bytes_kib:-0}" "${files:-0}"
}

# --- one downloaded title, so "data kept" has something to keep -------------
#
# The album page is opened by the app's OWN link (`neezer://album/<id>`, MB2's
# route table): one intent instead of a walk through the library's leaf strip,
# whose chips are horizontally scrolled and whose playlist names are random.
# The album is the bench's own (`bench_album` above) and has titles, so the page
# draws the same `DownloadButton` a person taps.
step "one title downloaded, before the upgrade"
adb_dev shell am start -W -a android.intent.action.VIEW -d "neezer://album/$album_id" >/dev/null
await_text "Télécharger" 30 || fail_screen "the album page did not paint its « Télécharger » action"
tap_text "Télécharger" || fail_screen "the « Télécharger » action is not tappable"
# The button reads « 0 / 2 » while the set is queued: the queue itself is the
# proof that the tap landed, and everything below names WHY it did not move.
# A download that makes no progress is either a request the server never saw
# (nothing in its access log beyond the set's resolve), one it refused (`503` +
# `Retry-After`, the background admission), or one the phone never issued at
# all — and the three have nothing in common but the screen. So the failure
# path reads the three places that answer: the server's own log, the DEVICE's
# threads (`nz-offline` is the worker: absent means it never started), and
# Réglages → Téléchargements, the one screen that prints the worker's sentence
# and its note in French (rule 2: « en file », « hors ligne », « en pause »,
# « aucune session » are all the same empty button to a person).
diagnose_download() {
    local pid
    pid="$(app_pid)"
    printf 'the app threads (nz-offline is the downloads worker):\n' >&2
    if [[ -n "$pid" ]]; then
        adb_dev shell ps -T -p "$pid" 2>/dev/null |
            awk 'NR == 1 || /nz-|offline/' >&2 || true
    else
        printf 'the app has no process\n' >&2
    fi
    printf 'the server answered (last 25 requests):\n' >&2
    grep -E '"(GET|POST) /' "$access_log" | tail -n 25 >&2 || true
    if tap_text "Réglages" && await_text "Chercher un réglage" 10 &&
        tap_text "Téléchargements" && await_text "Qualité de téléchargement par défaut" 10; then
        # The CAUSE of each failure is a sentence at the bottom of the queue card
        # (`SetRowView`) — below the fold, and `screen_texts`' 30-node window ends
        # before it. Scroll to the card's own retry button and print EVERY text.
        scroll_to_text "Tout réessayer" 5 || true
        printf 'the downloads section says:\n' >&2
        ui_text | tr '>' '\n' | sed -n 's/.* text="\([^"]\+\)".*/\1/p' |
            head -60 | paste -sd '|' - >&2
        printf '\n' >&2
    else
        printf 'could not open Réglages → Téléchargements (screen: %s)\n' "$(screen_texts)" >&2
    fi
    # The reserved `.part` and the published files, as root: a download that
    # failed before its first byte is a path or a disk, and this names which.
    if ensure_root; then
        printf 'the app files dir:\n' >&2
        adb_dev shell "ls -la /data/data/$ui_package/files 2>&1 | head -20" >&2 || true
        adb_dev shell "df -k /data | tail -2" >&2 || true
        # The store's OWN record. `downloads.error` is the sentence the worker
        # wrote when it gave up — and this release's card draws NOTHING for it
        # (measured: « 0 / 2 · 2 échec(s) » then the note then « Tout réessayer »,
        # with no cause line between), so the screen cannot answer this one.
        #
        # The file is `nz-cache`'s `desktop.sqlite` under `files/neezer` (its
        # OWN name, not `*.db`: a `-name '*.db'` search found nothing and cost a
        # dispatch), so the path is named here and `find` only widens it.
        printf 'the core data dir:\n' >&2
        adb_dev shell "ls -la /data/data/$ui_package/files/neezer 2>&1 | head -12" >&2 || true
        db="/data/data/$ui_package/files/neezer/desktop.sqlite"
        adb_dev shell "test -f $db" >/dev/null 2>&1 ||
            db="$(adb_dev shell "find /data/data/$ui_package -name '*.sqlite' 2>/dev/null | head -1" | tr -d '\r')"
        if [[ -n "$db" ]]; then
            printf 'the downloads table (%s):\n' "$db" >&2
            # The WAL, not the main file: the app keeps its connection open, so
            # `desktop.sqlite` is 4 KiB and EVERY row is in `-wal` (601 KiB on
            # dispatch 45760712206). Pulling the trio is what makes the query
            # see the data; the main file alone answers "no such table".
            adb_dev pull "$db" "$work/cache.sqlite" >/dev/null 2>&1 || true
            adb_dev pull "$db-wal" "$work/cache.sqlite-wal" >/dev/null 2>&1 || true
            adb_dev pull "$db-shm" "$work/cache.sqlite-shm" >/dev/null 2>&1 || true
            python3 - "$work/cache.sqlite" <<'PY' >&2 || true
import sqlite3, sys
conn = sqlite3.connect(sys.argv[1])
tables = [row[0] for row in conn.execute(
    "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")]
print("  tables:", ", ".join(tables) or "(none)")
try:
    rows = conn.execute("SELECT track, quality, state, error FROM downloads").fetchall()
except sqlite3.Error as error:
    print(f"  sqlite: {error}")
    raise SystemExit(0)
print(f"  {len(rows)} download row(s)")
for track, quality, state, error in rows:
    print(f"  {track} {quality} {state} :: {error}")
PY
        else
            printf 'no .sqlite under /data/data/%s\n' "$ui_package" >&2
        fi
    fi
}

downloaded=0
for round in 1 2 3; do
    if await_text "Téléchargé" 60; then
        downloaded=1
        break
    fi
    printf 'the button still reads (round %s/3): %s\n' "$round" "$(screen_texts)" >&2
done
if (( downloaded == 0 )); then
    diagnose_download
    fail_screen "the download of album $album_id did not finish in 180 s (the button never said « Téléchargé »)"
fi
printf 'downloaded=1 album=%s\n' "$album_id"
check_alive "the download"

ensure_root || fail_screen "adb root refused: this image cannot show the app's data directory, so data_kept cannot be measured"
read -r previous_bytes_kib previous_files <<<"$(data_measure)"
(( previous_files > 0 )) || fail_screen "the app's data directory holds no file after a sign-in and a download"
printf 'before_upgrade data_kib=%s data_files=%s\n' "$previous_bytes_kib" "$previous_files"

# --- the install over it ----------------------------------------------------

step "this build, installed over the previous one"
installed_code="$(adb_dev shell dumpsys package "$ui_package" | sed -n 's/^ *versionCode=\([0-9]*\).*/\1/p' | head -1 | tr -d '\r')"
[[ -n "$installed_code" ]] || fail_screen "dumpsys package printed no versionCode for $ui_package"
[[ "$installed_code" == "$previous_code" ]] ||
    die "the device holds versionCode $installed_code, not the previous APK's $previous_code: the old install did not take"
# `-r` is the whole point: replace, keep the data.
if ! out="$(adb_dev install -r -g "$to" 2>&1)"; then
    printf '%s\n' "$out" >&2
    fail_screen "the new APK did not install over the previous one"
fi
printf '%s\n' "$out" | tail -2

current_installed="$(adb_dev shell dumpsys package "$ui_package" | sed -n 's/^ *versionCode=\([0-9]*\).*/\1/p' | head -1 | tr -d '\r')"
last_update="$(adb_dev shell dumpsys package "$ui_package" | sed -n 's/^ *lastUpdateTime=//p' | head -1 | tr -d '\r')"
printf 'installed_version_code=%s last_update=%s\n' "$current_installed" "$last_update"
[[ "$current_installed" == "$current_code" ]] ||
    fail_screen "the device holds versionCode $current_installed, not this build's $current_code"
(( current_installed > previous_code )) ||
    fail_screen "the installed versionCode did not increase ($previous_code → $current_installed)"
printf 'version_code_up=1 previous=%s current=%s\n' "$previous_code" "$current_installed"

# --- the session survived ---------------------------------------------------

step "the session, after the install"
before_covers="$(covers_now)"
before_me="$(me_now)"
adb_dev shell am start -W -n "$(nz_launcher_component "$serial")" >/dev/null
await_text "Accueil Neezer" 30 || fail_screen "the home never painted after the upgrade: the session did not survive"
# Neither setup screen may be anywhere: the address form means no server, the
# account form means a session that was forgotten.
for forbidden in "Adresse du serveur" "Ton compte"; do
    if ui_text | grep -qF -- "$forbidden"; then
        fail_screen "« $forbidden » is on screen after the upgrade: the session was lost"
    fi
done
started=$SECONDS
after_me="$before_me"
while (( SECONDS - started < 60 )); do
    [[ -n "$(app_pid)" ]] || fail_screen "the app died after the upgrade"
    after_me="$(me_now)"
    (( after_me > before_me )) && break
done
(( after_me > before_me )) ||
    fail_screen "the home painted, but no authenticated request was served in 60 s (GET /api/me 200: before=$before_me after=$after_me): the stored session is not working"
printf 'session_kept=1 me_before=%s me_after=%s covers_before=%s covers_after=%s\n' \
    "$before_me" "$after_me" "$before_covers" "$(covers_now)"
check_alive "the home after the upgrade"

# --- the downloaded title survived ------------------------------------------

step "the downloaded title, after the install"
adb_dev shell am start -W -a android.intent.action.VIEW -d "neezer://album/$album_id" >/dev/null
await_text "Téléchargé" 30 || fail_screen "album $album_id no longer says « Téléchargé » after the upgrade: the offline store did not survive"
printf 'offline_kept=1 album=%s\n' "$album_id"

read -r current_bytes_kib current_files <<<"$(data_measure)"
printf 'after_upgrade data_kib=%s data_files=%s\n' "$current_bytes_kib" "$current_files"
(( current_files >= previous_files )) ||
    fail_screen "the app's data directory lost files across the install ($previous_files → $current_files)"
(( current_bytes_kib >= previous_bytes_kib )) ||
    fail_screen "the app's data shrank across the install ($previous_bytes_kib → $current_bytes_kib KiB)"
printf 'data_kept=1 data_files_before=%s data_files_after=%s data_kib_before=%s data_kib_after=%s\n' \
    "$previous_files" "$current_files" "$previous_bytes_kib" "$current_bytes_kib"

# --- the negative control, asserted -----------------------------------------
#
# A SECOND APK with the same package and another signer, installed over the one
# just upgraded. Android must refuse it with `INSTALL_FAILED_UPDATE_INCOMPATIBLE`;
# a run that "passes" because the install succeeded proves the control is not a
# control. The debug APK carries the SAME versionCode on purpose (the job builds
# it with the same `VERSION_CODE`): with versionCode 1 Android would answer
# `INSTALL_FAILED_VERSION_DOWNGRADE` first, and the signer would never be
# examined.
step "the negative control: a different signer must be refused"
control_signer="$(apk_signer_sha256 "$debug_apk")"
[[ -n "$control_signer" ]] || fail_screen "the debug APK carries no readable signer"
[[ "$control_signer" != "$current_signer" ]] ||
    fail_screen "the debug APK is signed with the RELEASE key: it cannot be the negative control"
if control_out="$(adb_dev install -r "$debug_apk" 2>&1)"; then
    printf '%s\n' "$control_out" >&2
    fail_screen "a differently-signed APK installed over the release one: the control proved nothing"
fi
printf '%s\n' "$control_out" | tail -3
grep -q "INSTALL_FAILED_UPDATE_INCOMPATIBLE" <<<"$control_out" || {
    printf '%s\n' "$control_out" >&2
    fail_screen "the install was refused, but not for the signer (expected INSTALL_FAILED_UPDATE_INCOMPATIBLE)"
}
printf 'negative_control=INSTALL_FAILED_UPDATE_INCOMPATIBLE expected=1\n'

# The refusal must have changed nothing: the same build is still the installed one.
still_installed="$(adb_dev shell dumpsys package "$ui_package" | sed -n 's/^ *versionCode=\([0-9]*\).*/\1/p' | head -1 | tr -d '\r')"
[[ "$still_installed" == "$current_code" ]] ||
    fail_screen "the refused install changed the installed versionCode ($current_code → $still_installed)"

printf 'upgrade=ok previous=%s current=%s data_kept=1 session_kept=1 version_code_up=1\n' \
    "$from" "$current_name"
