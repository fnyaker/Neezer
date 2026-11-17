#!/usr/bin/env bash
# mobile/tools/env.sh — the environment the mobile tools share. SOURCE it, never
# run it:  . "$(dirname "$0")/env.sh"
#
# It exists because three of those settings cost an afternoon each when a lot
# guesses them:
#
#  - the SDK is this machine's `~/.local/share/android-sdk` (never /tmp: it is a
#    RAM tmpfs), and a CI runner names it ANDROID_SDK_ROOT instead;
#  - Gradle 8.14.7 refuses a JDK 25 with `java.lang.IllegalArgumentException:
#    25.0.5` and no mention of Java at all — AGP 8.13 is built for JDK 17-24, and
#    this machine's `java` is 25/27. So the JDK is RESOLVED here, once;
#  - the Gradle launcher lives in `~/.local/share/gradle/gradle-8.14.7` (there is
#    no wrapper in the repository, deliberately: one pin, in the workflow).

nz_repo_root() {
    local here
    here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    (cd "$here/../.." && pwd)
}

nz_sdk_root() {
    if [[ -n "${ANDROID_SDK_ROOT:-}" ]]; then
        printf '%s\n' "$ANDROID_SDK_ROOT"
    elif [[ -n "${ANDROID_HOME:-}" ]]; then
        printf '%s\n' "$ANDROID_HOME"
    else
        printf '%s\n' "$HOME/.local/share/android-sdk"
    fi
}

nz_adb() { printf '%s\n' "$(nz_sdk_root)/platform-tools/adb"; }

nz_android_project() { printf '%s\n' "$(nz_repo_root)/mobile/android"; }

# The major version in <jdk>/release, or nothing.
nz_java_major() {
    local release="$1/release"
    [[ -r "$release" ]] || return 1
    sed -n 's/^JAVA_VERSION="\([0-9][0-9]*\).*/\1/p' "$release" | head -1
}

# The JDK Gradle and AGP actually accept (17-24), in preference order:
# the caller's JAVA_HOME when it is one, then mise's java@21, then a ~/.jdks 21,
# then the system java. Nothing accepted is a hard error naming the fix: a build
# that dies on "25.0.5" tells nobody what to do.
nz_java_home() {
    local candidate major
    local -a candidates=()
    [[ -n "${JAVA_HOME:-}" ]] && candidates+=("$JAVA_HOME")
    if command -v mise >/dev/null 2>&1; then
        candidate="$(mise where java@21 2>/dev/null || true)"
        [[ -n "$candidate" ]] && candidates+=("$candidate")
    fi
    for candidate in "$HOME"/.local/share/mise/installs/java/21*/ \
        "$HOME"/.local/share/opencode-v2-home/mise/installs/java/21*/ \
        "$HOME"/.jdks/*21*/ /usr/lib/jvm/*21*/; do
        [[ -d "$candidate" ]] && candidates+=("${candidate%/}")
    done
    if command -v java >/dev/null 2>&1; then
        candidate="$(dirname "$(dirname "$(readlink -f "$(command -v java)")")")"
        [[ -d "$candidate" ]] && candidates+=("$candidate")
    fi

    for candidate in "${candidates[@]}"; do
        major="$(nz_java_major "$candidate" || true)"
        [[ -n "$major" ]] || continue
        if (( major >= 17 && major <= 24 )); then
            printf '%s\n' "$candidate"
            return 0
        fi
    done

    printf 'env.sh: no JDK 17-24 found (JAVA_HOME=%s); AGP 8.13 refuses JDK 25+ with an opaque "25.0.5".\n' \
        "${JAVA_HOME:-unset}" >&2
    printf 'env.sh: install one with `mise install java@21`.\n' >&2
    return 1
}

nz_gradle() {
    if [[ -n "${GRADLE:-}" ]]; then
        printf '%s\n' "$GRADLE"
    elif [[ -x "$HOME/.local/share/gradle/gradle-8.14.7/bin/gradle" ]]; then
        printf '%s\n' "$HOME/.local/share/gradle/gradle-8.14.7/bin/gradle"
    elif command -v gradle >/dev/null 2>&1; then
        command -v gradle
    else
        printf 'env.sh: no gradle: install 8.14.7 under ~/.local/share/gradle (the version mobile.yaml pins)\n' >&2
        return 1
    fi
}

# Runs Gradle for the Android project with the resolved JDK and the SDK, from the
# project directory (a task path like :baselineprofile:… is relative to it).
nz_gradle_android() {
    local java_home
    java_home="$(nz_java_home)" || return 1
    (
        cd "$(nz_android_project)" || exit 1
        JAVA_HOME="$java_home" \
            ANDROID_HOME="$(nz_sdk_root)" \
            ANDROID_SDK_ROOT="$(nz_sdk_root)" \
            PATH="$java_home/bin:$PATH" \
            exec "$(nz_gradle)" --console=plain "$@"
    )
}

# The app's launcher component in the `pkg/.Activity` form `am start -n` takes,
# or nothing when the app is not installed on <serial>.
#
# `--brief` matters: without it `cmd package resolve-activity` prints a resolution
# dump (the activity's flags, its createTimestamp, the Application class) and the
# component is buried in it; a script that greps for `name=` then picks the wrong
# line on the next Android version. Measured on API 35: `--brief` answers exactly
# two lines, the second being `org.nsupysonic.app/.SetupActivity`.
nz_launcher_component() {
    local serial="$1"
    "$(nz_adb)" -s "$serial" shell cmd package resolve-activity --brief \
        -c android.intent.category.LAUNCHER org.nsupysonic.app 2>/dev/null |
        tail -1 | tr -d '\r'
}
