#!/usr/bin/env bash
# mobile/tools/bughunt/explore.sh — LA CHASSE AUX BUGS NATIVE : le cycle de vie de
# l'appareil, puis le marcheur (`explore.py`). Plan `plan-bughunt`, lot B2.
#
#   mobile/tools/bughunt/explore.sh --serial emulator-5580          # les trois scénarios + la passe
#   mobile/tools/bughunt/explore.sh --serial emulator-5580 --scenario burst
#   mobile/tools/bughunt/explore.sh --serial emulator-5580 --apk /tmp/neezer-native-1.6.6.apk
#   mobile/tools/bughunt/explore.sh --serial R58… --no-start         # un vrai téléphone
#   mobile/tools/bughunt/explore.sh --serial emulator-5580 --only "Réglages"
#
# Ce que ce script possède (et `explore.py` non) : l'appareil. Il démarre
# l'émulateur par `emu.sh` (jamais un qemu orphelin, jamais une fenêtre, jamais
# le son), attend `sys.boot_completed`, prépare l'appareil (`ui.sh`), installe un
# APK **release signé**, se connecte avec `release-signin.sh` (la même connexion
# que T1 : un crawler sur l'écran de setup n'explore qu'un écran), puis passe la
# main au marcheur. Un `trap` arrête l'émulateur qu'il a démarré, quoi qu'il
# arrive — et seulement celui-là (`emu.sh` ne signale que le PID qu'il a écrit).
#
# Ce qu'il REFUSE (plan §9.7) :
#   - toucher un appareil qu'il n'a pas démarré : chaque appel est
#     `adb -s <serial>`, jamais `adb` nu, et `--no-start` nomme l'appareil ;
#   - installer un APK debug : `--apk` doit être un release signé, et le script
#     imprime `apk=<nom> bytes=<n> signer=<sha256 court>` pour que le rapport
#     prouve QUEL fichier a été chassé ;
#   - tourner sans `hide_error_dialogs=1` : `prepare_device` le pose, sinon un
#     dialogue d'ANR du SystemUI décrit l'écran à la place de l'app ;
#   - cliquer une action destructive au-delà de sa confirmation : c'est
#     `explore.py` qui le tient (§4.5), une seule liste pour les deux.
#
# Une passe complète (sans `--scenario`) joue, dans cet ordre : la rafale
# « Suivant » (le premier symptôme du propriétaire), l'application de chaque
# réglage, la passe, puis la déconnexion/reconnexion — en dernier, parce qu'elle
# laisse l'app déconnectée jusqu'à ce que ce script la reconnecte.
#
# Sortie 0 quand la chasse a tourné (même si elle a trouvé des bugs : un bug est
# un RÉSULTAT), 1 quand un plantage a été enregistré (le job doit le voir), 2 sur
# une erreur d'usage, un APK debug, ou une connexion impossible (§9.11 : ce n'est
# pas un résultat de chasse).

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=mobile/tools/env.sh disable=SC1091
. "$here/../env.sh"

serial=""
apk=""
out=""
only=""
scenario=""
max_nodes=700
max_minutes=30
start=1
while [[ $# -gt 0 ]]; do
    case "$1" in
        --serial) serial="$2"; shift 2 ;;
        --serial=*) serial="${1#*=}"; shift ;;
        --apk) apk="$2"; shift 2 ;;
        --apk=*) apk="${1#*=}"; shift ;;
        --out) out="$2"; shift 2 ;;
        --out=*) out="${1#*=}"; shift ;;
        --only) only="$2"; shift 2 ;;
        --only=*) only="${1#*=}"; shift ;;
        --scenario) scenario="$2"; shift 2 ;;
        --scenario=*) scenario="${1#*=}"; shift ;;
        --max-nodes) max_nodes="$2"; shift 2 ;;
        --max-nodes=*) max_nodes="${1#*=}"; shift ;;
        --max-minutes) max_minutes="$2"; shift 2 ;;
        --max-minutes=*) max_minutes="${1#*=}"; shift ;;
        --no-start) start=0; shift ;;
        -h | --help) sed -n '2,44p' "$0"; exit 0 ;;
        *) printf 'explore.sh: unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done

[[ -n "$serial" ]] || { printf 'explore.sh: --serial is required\n' >&2; exit 2; }
case "$scenario" in
    "" | burst | settings | session) ;;
    *) printf 'explore.sh: --scenario must be burst, settings or session (got %s)\n' "$scenario" >&2; exit 2 ;;
esac
[[ "$max_nodes" =~ ^[1-9][0-9]*$ ]] || { printf 'explore.sh: --max-nodes must be a positive integer\n' >&2; exit 2; }
[[ "$max_minutes" =~ ^[1-9][0-9]*$ ]] || { printf 'explore.sh: --max-minutes must be a positive integer\n' >&2; exit 2; }

python3 - <<'PY' || { printf 'explore.sh: python3 >= 3.10 is required\n' >&2; exit 2; }
import sys
raise SystemExit(0 if sys.version_info >= (3, 10) else 1)
PY

note() { printf 'explore.sh: %s\n' "$*" >&2; }
step() { printf '\n=== %s\n' "$*"; }

# `mobile/tools/bughunt/out`, the path `bughunt.yaml`'s `native` job uploads as
# the `bughunt-native` artifact and `.gitignore` already covers (plan §4.7). A
# `mktemp` default would leave the job with nothing to upload, and
# `if-no-files-found: warn` would hide it.
if [[ -z "$out" ]]; then
    out="$here/out"
fi
mkdir -p "$out"
note "preuves: $out"

# NOT `started`: `ui.sh`'s `await_text` declares `local started=$SECONDS`, and a
# `local` over a READONLY global fails the whole function (measured on run
# 45904029041: await_text returned 1 in 1.5 s and the sign-in aborted).
readonly script_started=$SECONDS
port=""
if [[ "$serial" == emulator-* ]]; then
    port="${serial#emulator-}"
fi

# The bench server the HUNT talks to. `release-signin.sh` starts its own and
# stops it on exit (its `cleanup`, T1's choice, documented in
# `release-journey.sh`: every step after it runs offline), so a pass that relies
# on it explores offline shells only — measured on run 45898606608:
# « Réessayer » on Bibliothèque, « Les podcasts n'ont pas pu être chargés », and
# a 「 Suivant 」 burst that never played a track. The hunt therefore keeps its
# OWN server (the plan's 8 000-title bench, `--covers`) alive for its whole run,
# and reconnects the app to it after the release sign-in has proved the artifact.
bench_dir=""
bench_state=""
bench_port=""
t1_done=0

# `emu.sh` ne signale que le PID qu'il a écrit : l'arrêt ne peut pas tuer
# l'émulateur d'un autre lot, et un `--no-start` n'arrête rien.
# shellcheck disable=SC2329  # invoked through `trap cleanup EXIT`
cleanup() {
    local code=$?
    if [[ -n "$bench_state" ]]; then
        NEEZER_PERF_STATE="$bench_state" "$here/../perf-server.sh" stop >/dev/null 2>&1 || true
        [[ -z "$bench_dir" ]] || rm -rf "$bench_dir"
    fi
    if [[ -n "$port" && "$start" == 1 ]]; then
        "$here/../emu.sh" stop --port "$port" >/dev/null 2>&1 || true
    fi
    exit "$code"
}
trap cleanup EXIT

# shellcheck source=mobile/tools/ui.sh disable=SC1091
. "$here/../ui.sh"

boot_seconds=0
if [[ -n "$port" && "$start" == 1 ]]; then
    step "emulator $serial"
    "$here/../emu.sh" create
    "$here/../emu.sh" start --port "$port"
    boot_line="$("$here/../emu.sh" wait --port "$port")"
    note "$boot_line"
    boot_seconds="${boot_line##*seconds=}"
else
    note "appareil déjà démarré ($serial) : ce script n'en démarre et n'en arrête aucun"
fi
printf 'boot_seconds=%s\n' "$boot_seconds"

step "l'appareil qu'une personne trouverait"
prepare_device
check_alive "le démarrage"

# --- l'APK : celui qu'une personne installe ------------------------------------

resolve_apk() {
    [[ -n "$apk" ]] && return 0
    local candidate
    for candidate in \
        "$(nz_android_project)/app/build/outputs/apk/release/app-universal-release.apk" \
        "$(nz_android_project)/app/build/outputs/apk/release/app-release.apk"; do
        if [[ -f "$candidate" ]]; then
            apk="$candidate"
            return 0
        fi
    done
    return 1
}

apksigner_bin() {
    # `command -v` first: the SDK's build-tools may be absent while apksigner is
    # on the PATH (measured on the first CI run, 26/11: `signer=unknown`). Then
    # every known SDK root, the one Gradle uses included.
    local candidate latest root
    candidate="$(command -v apksigner 2>/dev/null || true)"
    if [[ -n "$candidate" ]]; then
        printf '%s\n' "$candidate"
        return 0
    fi
    for root in "$(nz_sdk_root)" "${ANDROID_HOME:-}" "${ANDROID_SDK_ROOT:-}" "$HOME/Android/Sdk"; do
        [[ -d "$root/build-tools" ]] || continue
        latest="$(find "$root/build-tools" -maxdepth 2 -name apksigner -type f 2>/dev/null |
            sort -V | tail -1)"
        if [[ -n "$latest" ]]; then
            printf '%s\n' "$latest"
            return 0
        fi
    done
    printf '\n'
}

step "installer l'APK release"
resolve_apk || note "aucun APK : le workflow le passe en --apk, localement \`gradle -p mobile/android :app:assembleRelease\`"
[[ -n "$apk" && -f "$apk" ]] || { note "aucun APK à installer"; exit 2; }
signer=""
apksigner="$(apksigner_bin)"
if [[ -n "$apksigner" ]]; then
    if ! certs="$("$apksigner" verify --print-certs "$apk" 2>&1)"; then
        # Never silent: on the first CI run the signer read `unknown` and the
        # cause was nowhere in the log.
        note "apksigner ($apksigner) n'a pas lu $apk : $(head -1 <<<"$certs")"
        certs=""
    fi
    signer="$(sed -n 's/^Signer #1 certificate SHA-256 digest: //p' <<<"$certs" | head -1 | cut -c1-16)"
    if grep -q 'CN=Android Debug' <<<"$certs"; then
        note "$apk est signé par la clé DEBUG : une chasse ne mesure pas un APK debug (plan §9.7)"
        exit 2
    fi
else
    note "apksigner introuvable (PATH ni sous $(nz_sdk_root)/build-tools) : le signataire ne sera pas prouvé"
fi
bytes="$(stat -c %s "$apk")"
apk_sha="$(sha256sum "$apk" | cut -c1-16)"
apk_name="$(basename "$apk")"
# The three facts that say WHICH file was hunted: its name, its size and the
# signature it carries. `bughunt.yaml` does not export the `mobile.yaml` run id
# to us, so the sha256 is the provenance anchor a reader can compare by hand.
printf 'apk=%s bytes=%s signer=%s sha256=%s\n' "$apk_name" "$bytes" "${signer:-unknown}" "$apk_sha"
adb_dev install -r -g "$apk" 2>&1 | tail -2
nz_launcher_component "$serial" | grep -qF 'fr.neezer.mobile/' ||
    { note "fr.neezer.mobile ne résout pas son lanceur après l'installation de $apk"; exit 1; }

# --- la connexion (la même que T1), et le serveur qui reste ------------------

bench_start() {
    [[ -z "$bench_port" ]] || return 0
    bench_dir="$(mktemp -d)"
    bench_state="$bench_dir/perf"
    step "le serveur de banc de la chasse (8 000 titres, TLS jetable)"
    openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj "/CN=neezer-bughunt" \
        -addext "subjectAltName=IP:10.0.2.2,IP:127.0.0.1" \
        -keyout "$bench_dir/key.pem" -out "$bench_dir/cert.pem" >/dev/null 2>&1
    local server_out
    server_out="$(NEEZER_PERF_STATE="$bench_state" "$here/../perf-server.sh" start \
        --covers "$(nz_repo_root)/tests/assets/cover.jpg" \
        --tls-cert "$bench_dir/cert.pem" --tls-key "$bench_dir/key.pem")" || return 1
    bench_port="$(sed -n 's/^port=\([0-9]*\) .*/\1/p' <<<"$server_out")"
    [[ -n "$bench_port" ]] || { note "le serveur de banc n'a pas de port: $server_out"; return 1; }
    printf 'bench_server=10.0.2.2:%s\n' "$bench_port"
}

# The setup flow release-signin.sh runs, against the server this script owns:
# address, certificate check OFF (the bench's leaf is self-signed and accepted on
# screen), the account the bench fixture publishes (bench / Bench1). The tree is
# T1's (`type_into`, `tap_text`, `await_text`), not a second walker.
own_signin() {
    # The server FIRST: `bench_port` is only known once it has started
    # (measured on run 45905460989: the address was typed as "10.0.2.2:").
    bench_start || return 1
    local addr="10.0.2.2:${bench_port}"
    # `ui_package`: T1's `mobile/tools/ui.sh`, sourced above (shellcheck cannot
    # follow the `../ui.sh` path, the `source=` hint names `mobile/tools/ui.sh`).
    # shellcheck disable=SC2154
    adb_dev shell pm clear "$ui_package" >/dev/null 2>&1 || true
    adb_dev shell am start -W -n "$(nz_launcher_component "$serial")" >/dev/null 2>&1 || true
    await_text "Adresse du serveur" 30 || { note "l'écran serveur ne peint pas"; return 1; }
    type_into 1 "$addr" || return 1
    tap_text "Vérifier le certificat" || { note "aucun commutateur « Vérifier le certificat »"; return 1; }
    tap_text "Vérifier le serveur" || { note "aucun bouton « Vérifier le serveur »"; return 1; }
    await_text "Certificat auto-signé" 30 || { note "pas de carte de certificat pour $addr"; return 1; }
    tap_text "Accepter ce certificat" || { note "aucun « Accepter ce certificat »"; return 1; }
    await_text "Ton compte" 30 || { note "le formulaire de compte ne s'ouvre pas"; return 1; }
    type_into 1 bench || return 1
    type_into 2 Bench1 secret || return 1
    tap_text "Se connecter" || { note "aucun bouton « Se connecter »"; return 1; }
    await_text "Accueil Neezer" 30 || { note "l'accueil ne peint pas après la connexion"; return 1; }
    printf 'signin=1 server=%s\n' "$addr"
}

signin() {
    step "connexion release"
    adb_dev logcat -c >/dev/null 2>&1 || true
    local log="$out/signin.log" ok=1
    if [[ "$t1_done" == 0 ]]; then
        # T1's own verdicts, once: they prove the RELEASE artifact signs in
        # (`release_tls_trusted=1`, `release_signin=1`). Its bench server dies
        # with it, so the app is then re-pointed at ours.
        "$here/../release-signin.sh" --serial "$serial" >"$log" 2>&1 || ok=0
        grep -E '^release_(tls_trusted|signin)=' "$log" || true
        t1_done=1
        if (( ok == 0 )); then
            note "la connexion release a ÉCHOUÉ : c'est un échec de démarrage, pas un résultat de chasse (§9.11)"
            sed 's/^/  /' "$log" >&2
            return 1
        fi
    fi
    own_signin || {
        note "la connexion au serveur de la chasse a échoué (pas un résultat de chasse, §9.11)"
        return 1
    }
}

# --- le marcheur ---------------------------------------------------------------

explore() {
    NEEZER_ADB="$(nz_adb)" \
    NEEZER_HEAD_SHA="${NEEZER_HEAD_SHA:-$(git -C "$(nz_repo_root)" rev-parse --short HEAD 2>/dev/null || echo unknown)}" \
    NEEZER_APK_NAME="$apk_name" \
    NEEZER_APK_BYTES="$bytes" \
    NEEZER_APK_SIGNER="${signer:-unknown}" \
    NEEZER_APK_SHA256="$apk_sha" \
        python3 "$here/explore.py" --serial "$serial" --out "$out" \
            --max-nodes "$max_nodes" --max-minutes "$max_minutes" "$@"
}

verdict_line() {
    python3 - "$out" "$1" <<'PY'
import json
import sys

path, key = sys.argv[1], sys.argv[2]
try:
    run = json.load(open(f"{path}/bughunt-native.json", encoding="utf-8"))
except (OSError, ValueError):
    raise SystemExit(0)
line = run.get("scenarios", {}).get(key, {}).get("line")
if line:
    print(line)
PY
}

status=0
if [[ -n "$only" ]]; then
    # `--only` rejoue un écran DÉJÀ découvert : la passe précédente a laissé
    # l'app connectée, et la liste des chemins dans son JSON. Pas de connexion,
    # pas de scénario — c'est le seul mode qui ne repart pas de zéro.
    explore --only "$only" || status=$?
    step "résumé"
    printf 'bughunt_seconds=%s\n' "$((SECONDS - script_started))"
    [[ -f "$out/run.txt" ]] && sed 's/^/  /' "$out/run.txt"
    printf 'bughunt_out=%s\n' "$out"
    exit "$status"
fi

case "$scenario" in
    burst | settings)
        signin || exit 1
        explore --scenario "$scenario" || status=$?
        ;;
    session)
        signin || exit 1
        explore --scenario session || status=$?
        session_line="$(verdict_line session)"
        [[ -n "$session_line" ]] && printf '%s\n' "$session_line"
        if signin; then
            printf 'session_roundtrip=ok login_painted=1 resignin=1 crash=0\n'
        else
            printf 'session_roundtrip=fail login_painted=%s resignin=0 crash=0\n' \
                "$(sed -n 's/.*login_painted=\([0-9]*\).*/\1/p' <<<"$session_line" | head -1)"
            status=1
        fi
        ;;
    *)
        # La passe complète : les trois scénarios, la passe, la session en dernier.
        signin || exit 1
        explore --scenario burst || status=$?
        explore --scenario settings || status=$?
        explore || status=$?
        explore --scenario session || status=$?
        session_line="$(verdict_line session)"
        [[ -n "$session_line" ]] && printf '%s\n' "$session_line"
        if signin; then
            printf 'session_roundtrip=ok login_painted=1 resignin=1 crash=0\n'
        else
            printf 'session_roundtrip=fail resignin=0\n'
            status=1
        fi
        ;;
esac

step "résumé"
printf 'bughunt_seconds=%s\n' "$((SECONDS - script_started))"
[[ -f "$out/run.txt" ]] && sed 's/^/  /' "$out/run.txt"
printf 'bughunt_out=%s\n' "$out"
exit "$status"
