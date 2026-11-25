#!/usr/bin/env bash
# mobile/tools/ui.sh — the accessibility-tree helpers the release scripts share.
# SOURCE it, never run it: the caller sets `serial` first and then does
#
#   . "$here/env.sh"
#   serial="emulator-5580"
#   . "$here/ui.sh"
#
# Why one file. `release-signin.sh` and `release-journey.sh` both walk the app the
# way a person does — by the nodes TalkBack reads — and a second copy of these
# helpers is a second behaviour: `release-journey.sh` taps the SAME tab bar, the
# same switch and the same Réglages row, so a fix to `tap_text` must reach both.
#
# The rules encoded here, each learned on a real run:
#
#  - a needle is the screen's OWN words. uiautomator writes the tree as XML
#    attributes, so `&` comes out `&amp;` and `'` as `&#39;`; the tree is
#    unescaped before every match, which is why a label containing an ampersand
#    (« Lecture & son ») is a valid needle at all (the older rule, "no apostrophe
#    in a needle", was this bug's symptom, not a design);
#  - a tap lands on the LAST node whose text is exactly the label: a tab's text
#    appears twice (the top bar's title, then the bar's own item) and the
#    clickable one is the bar's; tapping the title does nothing, and every door
#    after the first then fails for the wrong reason (measured on ci-emulator);
#  - a control below the fold is brought up by swipes the DEVICE performed, at
#    most five, never a sleep: uiautomator waits for idle, so a fixed wait is
#    both slower and flakier;
#  - the keyboard a field opened covers the lower half of the form, and a node
#    under it is not in the tree at all — BACK hides it, and only while it is
#    actually shown (a BACK with no keyboard leaves the screen);
#  - "the app is alive" is `pidof`, not a successful `am start`: an `am start`
#    that returns 0 proves a task was created, never that the process survived
#    the screen's first frame.

[[ -n "${serial:-}" ]] || {
    printf 'ui.sh: set serial before sourcing this file\n' >&2
    exit 2
}

# Not `package`: env.sh's functions declare a `local package`, which a readonly
# global of that name turns into an error.
ui_package=fr.neezer.mobile

adb_dev() { "$(nz_adb)" -s "$serial" "$@"; }

# --- the tree ---------------------------------------------------------------

# The accessibility tree (the nodes TalkBack reads), retried: `uiautomator dump`
# refuses while the UI is animating — it waits for idle and gives up — and a
# tab's ripple is not a failure.
dump_ui() {
    local attempt
    for attempt in 1 2 3 4 5; do
        if adb_dev shell uiautomator dump /sdcard/nz-ui.xml >/dev/null 2>&1; then
            adb_dev shell cat /sdcard/nz-ui.xml 2>/dev/null
            return 0
        fi
    done
    return 1
}

# The tree with its XML entities decoded, so a needle is the screen's own words.
ui_text() {
    dump_ui | sed -e 's/&quot;/"/g' -e "s/&#39;/'/g" -e 's/&lt;/</g' -e 's/&gt;/>/g' -e 's/&amp;/\&/g'
}

# Waits until a node whose text contains <needle> exists. 10 s for a tap's
# repaint (the default), 30 s for a cold launch — every caller passes its own.
await_text() {
    local needle="$1" budget="${2:-10}" started=$SECONDS
    while (( SECONDS - started < budget )); do
        if ui_text | grep -qF -- "$needle"; then
            return 0
        fi
    done
    return 1
}

# The device's current viewport, for a swipe that works on a phone as well as on
# the emulator (the old hardcoded 540x1600 was the pixel_6's size, nothing else).
ui_screen() {
    local width height
    read -r width height < <(adb_dev shell wm size | sed -n 's/.*: \([0-9]*\)x\([0-9]*\).*/\1 \2/p' | tail -1)
    printf '%s %s\n' "${width:-1080}" "${height:-2400}"
}

# Swipes up once: 70 % of the viewport, along the LEFT MARGIN. The margin matters:
# a vertical drag that starts on a control which owns vertical gestures (the
# equaliser's ten faders) moves the fader instead of the page — measured on the
# « Lecture & son » section, whose car card sits after five long effect cards.
ui_swipe_up() {
    local width height
    read -r width height < <(ui_screen)
    adb_dev shell input swipe $((width / 30)) $((height * 88 / 100)) \
        $((width / 30)) $((height * 18 / 100)) 240 >/dev/null
}

# Hides the keyboard when — and only when — it is shown.
hide_keyboard() {
    if adb_dev shell dumpsys input_method 2>/dev/null | grep -qE "mInputShown=true|isInputViewShown=true"; then
        adb_dev shell input keyevent KEYCODE_BACK >/dev/null
    fi
}

# The LAST node whose text is exactly <label>, tapped. A control below the fold is
# brought up by swipes the device performed — eight attempts at most, never a
# sleep. The tap lands on the node's UPPER part and not its centre: uiautomator
# reports bounds unclipped to the scroll container, so a target found at the
# bottom of a pane can have a centre under the tab bar, and a tap there hits the
# bar (`History`-length pages measured it).
tap_text() {
    local label="$1" bounds x1 y1 x2 y2 attempt width height
    hide_keyboard
    read -r width height < <(ui_screen)
    for attempt in 1 2 3 4 5 6 7 8; do
        bounds="$(ui_text | tr '>' '\n' | grep -F -- "text=\"$label\"" |
            sed -n 's/.*bounds="\[\([0-9]*\),\([0-9]*\)\]\[\([0-9]*\),\([0-9]*\)\]".*/\1 \2 \3 \4/p' | tail -1)"
        if [[ -n "$bounds" ]]; then
            read -r x1 y1 x2 y2 <<<"$bounds"
            if (( y1 < height - 12 )); then
                adb_dev shell input tap $(((x1 + x2) / 2)) $((y1 + 12)) >/dev/null
                return 0
            fi
        fi
        ui_swipe_up
    done
    return 1
}

# Swipes up until <needle> is on screen (a menu or a card longer than the
# window), then leaves it visible for tap_text. Bounded: <swipes> gestures, the
# caller names the count its page needs.
scroll_to_text() {
    local needle="$1" swipes="${2:-5}" attempt
    for attempt in $(seq 1 "$swipes"); do
        if ui_text | grep -qF -- "text=\"$needle\""; then
            return 0
        fi
        ui_swipe_up
    done
    ui_text | grep -qF -- "text=\"$needle\""
}

# Types <text> into the <n>-th text field of the screen (1-based, tree order):
# a Compose field is an EditText node. The field's own text is the proof — a
# field that has not taken the focus yet drops injected keys while `input text`
# reports success (MQ2's journey measured exactly that). The third argument is
# `secret` for a password field, which shows dots and never the text.
type_into() {
    local index="$1" text="$2" attempt bounds x1 y1 x2 y2
    for attempt in 1 2 3; do
        hide_keyboard
        bounds="$(ui_text | tr '>' '\n' | grep -F 'class="android.widget.EditText"' |
            sed -n 's/.*bounds="\[\([0-9]*\),\([0-9]*\)\]\[\([0-9]*\),\([0-9]*\)\]".*/\1 \2 \3 \4/p' |
            sed -n "${index}p")"
        [[ -n "$bounds" ]] || { printf 'ui.sh: no text field #%s on screen\n' "$index" >&2; return 1; }
        read -r x1 y1 x2 y2 <<<"$bounds"
        adb_dev shell input tap $(((x1 + x2) / 2)) $(((y1 + y2) / 2)) >/dev/null
        adb_dev shell input text "$text" >/dev/null
        # The password field shows dots, never the text: its proof is the login.
        if [[ "${3:-}" == secret ]] || ui_text | grep -qF -- "text=\"$text\""; then
            return 0
        fi
    done
    printf 'ui.sh: the field #%s never held « %s »\n' "$index" "$text" >&2
    return 1
}

# --- the app and the device -------------------------------------------------

app_pid() { adb_dev shell pidof "$ui_package" 2>/dev/null | tr -d '\r'; }

# WHY the app is not there. A native library that cannot be resolved, or a Kotlin
# exception at startup, leaves the screen black and the accessibility tree empty —
# the symptom a launch check reports alone ("« Accueil » never painted"), which
# names nothing actionable. The crash buffer names the cause.
app_crash_cause() {
    local crash
    # Two lines of context after the verdict: the exception class is what a
    # reader needs (`IllegalStateException: You need to use a Theme.AppCompat…`),
    # and the first frame says where.
    crash="$(adb_dev logcat -d -b crash 2>/dev/null |
        grep -E -A 2 "FATAL EXCEPTION|ANR in fr\.neezer\.mobile|UnsatisfiedLinkError|dlopen failed" |
        head -6 || true)"
    if [[ -n "$crash" ]]; then
        printf '%s' "$crash"
        return 0
    fi
    if [[ -z "$(app_pid)" ]]; then
        printf 'le processus %s ne tourne pas et le tampon de plantage est vide' "$ui_package"
    fi
}

# Every visible text of the screen, for a failure message that says what was there.
screen_texts() {
    ui_text | tr '>' '\n' | sed -n 's/.* text="\([^"]\+\)".*/\1/p' | head -30 | paste -sd '|' -
}

# The device a person would find: awake, past the keyguard, with the ANR/crash
# dialogs and the animations hushed. A freshly booted guest on a busy machine
# pops "System UI isn't responding" on top of the app, and every later dump then
# describes the DIALOG, not the screen (measured on ci-emulator); a locked screen
# would make every tap land on the keyguard and the walk would hang.
prepare_device() {
    adb_dev shell settings put global hide_error_dialogs 1 >/dev/null 2>&1 || true
    adb_dev shell settings put global window_animation_scale 0 >/dev/null 2>&1 || true
    adb_dev shell settings put global transition_animation_scale 0 >/dev/null 2>&1 || true
    adb_dev shell settings put global animator_duration_scale 0 >/dev/null 2>&1 || true
    adb_dev shell input keyevent KEYCODE_WAKEUP >/dev/null 2>&1 || true
    adb_dev shell wm dismiss-keyguard >/dev/null 2>&1 || true
}

# The emulator itself is a qemu process; if it dies mid-walk every later adb call
# answers "device not found" and the log would blame the wrong thing. A real
# phone (`--serial R58…`) has no PID file and is only checked through adb.
check_alive() {
    local where="$1" port pidfile pid
    if [[ "$serial" == emulator-* ]]; then
        port="${serial#emulator-}"
        pidfile="${NEEZER_EMU_STATE:-$HOME/.cache/neezer-emu}/$port.pid"
        if [[ -f "$pidfile" ]]; then
            pid="$(tr -dc '0-9' <"$pidfile")"
            if [[ -n "$pid" ]] && ! kill -0 "$pid" 2>/dev/null; then
                printf 'ui.sh: l emulateur est mort pendant %s; dernieres lignes (%s):\n' \
                    "$where" "${pidfile%.pid}.log" >&2
                tail -n 30 "${pidfile%.pid}.log" >&2 || true
                exit 1
            fi
        fi
    fi
    if [[ "$(adb_dev get-state 2>/dev/null || true)" != "device" ]]; then
        printf 'ui.sh: %s is not visible to adb during %s\n' "$serial" "$where" >&2
        exit 1
    fi
}

# The one failure exit: what was expected, what was on screen, the tree itself
# (bounded) and the crash cause. A job log is the only place these are ever read.
fail_screen() {
    local why="$1" cause
    printf 'ui.sh: %s\n' "$why" >&2
    printf 'screen: %s\n' "$(screen_texts)" >&2
    dump_ui | head -c 2000 >&2
    printf '\n' >&2
    cause="$(app_crash_cause)"
    [[ -n "$cause" ]] && printf 'cause: %s\n' "$cause" >&2
    exit 1
}
