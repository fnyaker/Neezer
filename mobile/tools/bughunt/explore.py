#!/usr/bin/env python3
"""The native bug hunt: a deterministic walker over the RELEASE APK's tree.

    mobile/tools/bughunt/explore.sh --serial emulator-5580        # the whole hunt
    python3 mobile/tools/bughunt/explore.py --self-test           # no emulator
    python3 mobile/tools/bughunt/explore.py --check-routes        # drift guard

`explore.sh` owns the DEVICE (boot, install, sign-in, cleanup); this file owns
the WALK. It knows nothing about `emu.sh`: it speaks `adb -s <serial>` and
nothing else, so it runs the same on a runner, on this machine, or against a
real phone through `--serial R58…`.

The contract is `plan-bughunt.md` §4 (both harnesses write the same vocabulary),
§9 (this lot) and `docs/agents/mobile.md` (*La chasse aux bugs native*). Three
rules it shares with the rest of `mobile/tools/`, each learned on a real run:

  - the tree is `uiautomator dump`, retried five times: it waits for idle and
    gives up while a ripple animates (the rule of `ui.sh`'s `dump_ui`, which a
    Python script cannot call — it is REIMPLEMENTED here, and a change to the
    retry rule touches both files in the same PR);
  - every `adb` call names the serial it was given (a bare `adb` may pick the
    person's phone);
  - "the app is alive" is `pidof`, and a crash record counts only when it names
    `fr.neezer.mobile` — the guest's crash buffer also collects SystemUI's and
    SwiftShader's, which are not this app's result (`ui.sh#app_crash_cause`).

The parser is NOT copied: `mobile/tools/judge/precheck.py` (T9) owns `Node`,
`parse_tree` and `dp()`, and this file loads it by path (§9.3). `precheck.Node`
does not carry `resource-id`, `checkable`, `long-clickable`, `checked` or
`selected`, which the element rule (§9.4) needs, so the same dump is read a
second time ONLY for its attribute list, with `precheck`'s own `TAG`/`ATTR`
regular expressions, zipped by document order: one parser, one truth.

A destructive action is clicked **up to** its confirmation and never beyond
(§4.5): the labels and the three regimes are written once below
(`DESTRUCTIVE`). An action whose regime is `fin` is never clicked during the
pass (`skipped-session`); a confirmation that never comes is a `bloquant` bug
(`confirm=no-confirm`), not a harness accident.

Nothing here fixes a bug: it records it. Exit status is 0 when the walk ran,
whatever it found; 1 when an exception was recorded (so a crash is visible to
the job); 2 on a usage error, a missing `precheck.py`, or a route drift.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import unicodedata
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path

APP = "native"
PKG = "fr.neezer.mobile"
SYSTEM_UI = "com.android.systemui"
TOOL = "mobile/tools/bughunt/explore.py"

# §4.1, in priority order: the first one that applies wins.
OUTCOMES = (
    "exception", "blank", "freeze", "console-error", "navigated",
    "opened-layer", "state-changed", "nothing",
    "blocked-destructive", "unreachable", "skipped-session",
)

# §4.6: a `nothing` (or an `unreachable`) on a control whose name starts with
# one of these is a blocking bug, not decoration. The owner's own words.
VERBS = (
    "Lire", "Ouvrir", "Ajouter", "Retirer", "Supprimer", "Appliquer", "Choisir",
    "Enregistrer", "Fermer", "Chercher", "Télécharger", "Suivant", "Précédent",
    "Lecture", "Réduire", "Partager", "Importer", "Créer", "Renommer", "Déplacer",
    "Vider", "Recharger", "Se connecter", "Se déconnecter",
)

# §4.5, native table. `regime` is `passe` (click up to the confirmation, then
# cancel), `fin` (never during the pass: `skipped-session`), `jamais` (click up
# to its sheet, never beyond) or `reversible` (no confirmation by design).
DESTRUCTIVE = {
    "Déconnexion": ("passe", False),
    "Changer de serveur": ("fin", False),
    "Supprimer le compte": ("passe", False),
    "Supprimer la playlist": ("passe", False),
    "Supprimer définitivement": ("passe", False),
    "Supprimer ce titre": ("passe", False),
    "Supprimer le dossier": ("passe", False),
    "Supprimer le marqueur": ("passe", False),
    "Vider la file": ("reversible", True),
    "Vider le cache": ("reversible", True),
    "Réinitialiser": ("reversible", True),
    "Mettre à jour": ("jamais", False),
}

# What a confirmation layer looks like, in words a person reads (§4.5).
CONFIRM_WORDS = ("Confirmer", "Annuler", "Supprimer", "Oui", "D'accord")

# Volatile parts of an identity (§9.4): an index in a resource id, a counter in
# a label ("· 3 titres"). They differ run to run for the same control.
VOLATILE_ID = re.compile(r":id/[^/\s\"]*\d[^/\s\"]*")
COUNTER = re.compile(r"·\s*\d+")

# §4.1, native `opened-layer`: « une fenêtre de plus dans `dumpsys window` ».
# The window manager prints one `Window{<hash> u<user> <name>}:` block per
# window; the name is what a second read compares. The system bars and the
# keyboard have their own verdicts (`foreign`, `ime`) and would turn any tap
# into a layer, so they are dropped here.
WINDOW = re.compile(r"Window\{[0-9a-f]+ u\d+ ([^}]+)\}")
WINDOW_SKIP = ("StatusBar", "NavigationBar", "NavigationBar0", "inputmethod")

# Evidence is bounded like the web harness's (§4.8: one screenshot per anomaly,
# ≤ 40). On the SwiftShader emulator a `screencap` costs ~1 s, and the pass has
# 54 `nothing` lines to prove: the 41st shot would only cost clicks.
ANOMALY_SHOTS = 40

# A dump slower than the first IS a freeze (§4.1: 3 000 ms). The second bounds
# what it costs to read the ANR buffer: only a slow or crashed tap pays it.
FREEZE_MS = 3000
ANR_CHECK_MS = 1200
# §4.1: `nothing` is « nothing for 3 000 ms (2 dumps) », never one read. A paint
# slower than the first dump reads as `nothing` on one pass and `state-changed`
# on the next (B1 measured five such lines at 1 200 ms alone), and a `nothing`
# on a verb-named control is a `bloquant` bug: an honest silence costs this wait.
NOTHING_WAIT_S = 2.0

TEXT_CLASSES = ("TextView", "Button", "ImageButton", "EditText",
                "CheckBox", "Switch", "RadioButton")

SCENARIO_LABEL = {
    "burst": "Rafale Suivant ×20 à 100 ms",
    "burst-alternated": "Rafale alternée ×20",
    "settings": "Appliquer chaque réglage",
    "session": "Déconnexion puis reconnexion",
}


# --------------------------------------------------------------------------
# precheck.py, loaded by path: T9 is its owner (§9.3)
# --------------------------------------------------------------------------

def load_precheck():
    """`mobile/tools/judge/precheck.py`, loaded by path: T9 en est le propriétaire."""
    import importlib.util

    path = Path(__file__).resolve().parents[1] / "judge" / "precheck.py"
    if not path.is_file():
        print("bughunt: precheck.py absent (T9 non fusionné)", file=sys.stderr)
        raise SystemExit(2)
    spec = importlib.util.spec_from_file_location("nz_precheck", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PRECHECK = None  # set by main() and by self_test()


# --------------------------------------------------------------------------
# small helpers
# --------------------------------------------------------------------------

def short_sha(text: str) -> str:
    # An IDENTITY key (§9.4), never a signature: `usedforsecurity=False` says so
    # in the hash's own terms, and the rule is silenced with that reason (the
    # shape `neezer/db.py` already uses).
    return hashlib.sha1(  # nosemgrep: python.lang.security.insecure-hash-algorithms.insecure-hash-algorithm-sha1 -- a control's identity, not a signature; see above
        text.encode("utf-8", "replace"), usedforsecurity=False
    ).hexdigest()[:16]


def slug(text: str, limit: int = 40) -> str:
    """The evidence-file slug (§4.7): transliterated, `[^a-z0-9]+` → `-`."""
    folded = unicodedata.normalize("NFKD", text).encode("ascii", "ignore").decode()
    cleaned = re.sub(r"[^a-z0-9]+", "-", folded.lower()).strip("-")
    return cleaned[:limit].strip("-") or "element"


def percent(clicked: int, found: int) -> str:
    return f"{100.0 * clicked / found:.1f}" if found > 0 else "0.0"


def starts_with_verb(name: str) -> bool:
    word = (name or "").strip()
    return any(word.startswith(verb) for verb in VERBS)


def parse_windows(text: str) -> tuple[str, ...]:
    """The windows a `dumpsys window windows` read names, system bars excluded.

    Duplicates are KEPT: a Compose dialog can carry the same name as the
    activity's own window, and `{name}` would fold the two into one — the count
    is what §4.1 measures (« une fenêtre de plus »).
    """
    names = [match.group(1).strip() for match in WINDOW.finditer(text or "")]
    skip = tuple(word.lower() for word in WINDOW_SKIP)
    return tuple(sorted(name for name in names
                        if name and not any(word in name.lower() for word in skip)))


def now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def git_sha() -> str:
    try:
        done = subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"],
            capture_output=True, text=True, timeout=10,
            cwd=Path(__file__).resolve().parents[3],
        )
        return done.stdout.strip() or "unknown"
    except (OSError, subprocess.SubprocessError):
        return "unknown"


# --------------------------------------------------------------------------
# adb: one serial, always
# --------------------------------------------------------------------------

class Adb:
    def __init__(self, serial: str):
        self.serial = serial
        self.bin = (
            os.environ.get("NEEZER_ADB")
            or shutil.which("adb")
            or str(Path.home() / ".local/share/android-sdk/platform-tools/adb")
        )

    def run(self, *args: str, timeout: int = 30) -> subprocess.CompletedProcess | None:
        try:
            return subprocess.run(
                [self.bin, "-s", self.serial, *args],
                capture_output=True, text=True, timeout=timeout,
            )
        except (OSError, subprocess.SubprocessError):
            return None

    def shell(self, *args: str, timeout: int = 30) -> str:
        done = self.run("shell", *args, timeout=timeout)
        return (done.stdout or "") if done else ""

    def tap(self, x: int, y: int) -> None:
        self.run("shell", "input", "tap", str(x), str(y), timeout=15)

    def key(self, code: str) -> None:
        self.run("shell", "input", "keyevent", code, timeout=15)

    def swipe_up(self, width: int, height: int) -> None:
        # The left margin, above the tab bar: the two choices `ui.sh` measured.
        self.run("shell", "input", "swipe", str(width // 30), str(height * 70 // 100),
                 str(width // 30), str(height * 30 // 100), "240", timeout=20)

    def screencap(self, path: Path) -> bool:
        try:
            with path.open("wb") as handle:
                done = subprocess.run(
                    [self.bin, "-s", self.serial, "exec-out", "screencap", "-p"],
                    stdout=handle, stderr=subprocess.DEVNULL, timeout=30,
                )
            return done.returncode == 0 and path.stat().st_size > 0
        except (OSError, subprocess.SubprocessError):
            return False

    def pid(self) -> str:
        return self.shell("pidof", PKG, timeout=10).strip()

    def windows(self) -> tuple[str, ...]:
        """§4.1: the windows on screen, read from the window manager (cheap)."""
        return parse_windows(self.shell("dumpsys", "window", "windows", timeout=20))

    def crash_lines(self) -> list[str]:
        """Only the crash records that name OUR package (the rule of monkey.sh)."""
        raw = self.run("logcat", "-d", "-b", "crash", timeout=20)
        if raw is None or raw.returncode != 0:
            return []
        blocks: list[list[str]] = [[]]
        for line in (raw.stdout or "").splitlines():
            if line.startswith("--------- beginning of"):
                blocks.append([])
                continue
            blocks[-1].append(line)
        named = [line for block in blocks if any(PKG in entry for entry in block) for line in block]
        return [line for line in named
                if "FATAL EXCEPTION" in line or "Fatal signal" in line
                or "UnsatisfiedLinkError" in line or "Exception" in line or "Error:" in line]

    def anr(self) -> list[str]:
        raw = self.run("logcat", "-d", "-b", "main", "-t", "300", timeout=20)
        if raw is None:
            return []
        return [line for line in (raw.stdout or "").splitlines() if f"ANR in {PKG}" in line]

    def launcher(self) -> str:
        out = self.shell("cmd", "package", "resolve-activity", "--brief",
                         "-c", "android.intent.category.LAUNCHER", PKG, timeout=15)
        for line in reversed(out.strip().splitlines()):
            if line.strip().startswith(PKG + "/"):
                return line.strip()
        return ""

    def device_description(self) -> dict[str, str]:
        def prop(name: str) -> str:
            return self.shell("getprop", name, timeout=10).strip()

        lines = self.shell("wm", "size", timeout=10).strip().splitlines()
        size = lines[-1].split(":", 1)[-1].strip() if lines else "1080x2400"
        model = prop("ro.product.model") or "unknown"
        sdk = prop("ro.build.version.sdk") or "unknown"
        return {
            "model": model,
            "sdk": sdk,
            "density": prop("ro.sf.lcd_density") or "420",
            "font_scale": self.shell("settings", "get", "system", "font_scale", timeout=10).strip() or "1.0",
            "size": size.replace("x", "×"),
            "avd": f"AVD {model} API {sdk}",
        }


# --------------------------------------------------------------------------
# the dump: `uiautomator dump` + `cat`, five attempts (ui.sh's rule)
# --------------------------------------------------------------------------

class Dump:
    """One screen, read once: the tree, its extra attributes, and its timing."""

    __slots__ = ("text", "ms", "root", "nodes", "wraps", "kids", "by_id")

    def __init__(self, text: str | None, ms: int):
        self.text = text
        self.ms = ms
        self.root = None
        self.nodes: list = []
        self.wraps: list["W"] = []
        self.kids: dict[int, list["W"]] = {}
        self.by_id: dict[int, "W"] = {}
        if text:
            self._parse()

    def _parse(self) -> None:
        scratch = Path(tempfile.gettempdir()) / f"bughunt-dump-{os.getpid()}.xml"
        try:
            scratch.write_text(self.text or "", encoding="utf-8")
            self.root, self.nodes = PRECHECK.parse_tree(scratch)
        except (OSError, ValueError):
            self.root, self.nodes = None, []
        finally:
            try:
                scratch.unlink()
            except OSError:
                pass
        if not self.nodes:
            return
        # The same scanner, read a second time for the attributes `precheck.Node`
        # does not keep. Document order is the parser's, so the zip is exact.
        raw = [
            {name: PRECHECK.decode(value) for name, value in PRECHECK.ATTR.findall(match.group(1))}
            for match in PRECHECK.TAG.finditer(self.text or "")
            if not match.group(0).startswith("</")
        ]
        if len(raw) != len(self.nodes):
            raw = [{} for _ in self.nodes]
        for node, attrs in zip(self.nodes, raw):
            wrap = W(node, attrs)
            self.wraps.append(wrap)
            self.by_id[id(node)] = wrap
            if node.parent is not None:
                self.kids.setdefault(id(node.parent), []).append(wrap)
        self._resolve_names()

    def _resolve_names(self) -> None:
        """The accessible name of a control whose label lives on a CHILD.

        Measured on the first real run (26/11, run 45895651634): a Compose tab
        is a clickable `View` with an empty `text`/`content-desc`, and its label
        is a child `TextView` (« Rechercher »). With the name read only from the
        node itself, the four tabs shared ONE signature, three were dropped as
        duplicates, and the pass never left the home screen
        (`screens=1 elements=9 duplicates=20`). The name is the node's own text,
        then its own desc, then the nearest descendant's text, then its desc —
        the short word a screen reader speaks.
        """
        for wrap in self.wraps:
            if not wrap.named:
                wrap.named = self._descendant_name(wrap, 3)

    def wrap_of(self, node) -> "W | None":
        """The `W` of a `precheck.Node`, for the attributes T9's Node drops."""
        return self.by_id.get(id(node))

    def _descendant_name(self, wrap: "W", depth: int) -> str:
        frontier = list(self.kids.get(id(wrap.n), []))
        for _ in range(depth):
            if not frontier:
                return ""
            for kid in frontier:
                if (kid.n.text or "").strip():
                    return kid.n.text.strip()
            for kid in frontier:
                if (kid.n.desc or "").strip():
                    return kid.n.desc.strip()
            below: list["W"] = []
            for kid in frontier:
                below.extend(self.kids.get(id(kid.n), []))
            frontier = below
        return ""

    # --- the semantic view a transition is judged on (§4.1) ----------------

    @property
    def ok(self) -> bool:
        return bool(self.nodes)

    @property
    def app_wraps(self) -> list["W"]:
        return [w for w in self.wraps if w.n.package == PKG]

    @property
    def texts(self) -> list[str]:
        return [w.word for w in self.app_wraps if w.word]

    @property
    def textual(self) -> int:
        return len(self.texts)

    @property
    def fingerprint(self) -> tuple:
        return tuple(sorted(w.state for w in self.app_wraps))

    @property
    def layer(self) -> tuple:
        return tuple(sorted(w.state for w in self.app_wraps if w.in_layer))

    @property
    def ime(self) -> bool:
        return any("inputmethod" in (w.n.package or "") for w in self.wraps)

    @property
    def foreign(self) -> tuple:
        """The other apps' windows on screen (a share sheet, a picker).

        Not ours, not the status bar, not the keyboard: a package that appears
        between two dumps IS a layer, and its absence is why « Partager » would
        otherwise read as `nothing` on a button that works.
        """
        return tuple(sorted({
            w.n.package for w in self.wraps
            if w.n.package and w.n.package not in (PKG, SYSTEM_UI)
            and "inputmethod" not in w.n.package
        }))

    @property
    def height(self) -> int:
        return self.root.y2 if self.root is not None else 2400

    @property
    def width(self) -> int:
        return self.root.x2 if self.root is not None else 1080

    @property
    def name(self) -> str:
        return screen_name(self)

    @property
    def hash(self) -> str:
        """The hash of the FIRST 20 texts (§9.5), not of the first 20 characters:
        truncating the joined string hashed every screen on its own title, so two
        screens with the same header read as one and the label collision never
        fired."""
        words = [w.word for w in self.app_wraps if w.word][:20]
        return hashlib.sha1(  # nosemgrep: python.lang.security.insecure-hash-algorithms.insecure-hash-algorithm-sha1 -- a screen's identity, not a signature
            "\u0001".join(words).encode("utf-8", "replace"), usedforsecurity=False
        ).hexdigest()[:8]

    def view(self) -> dict:
        return {
            "name": self.name,
            "hash": self.hash,
            "texts": self.texts[:20],
            "nodes": self.fingerprint,
            "textual": self.textual,
            "layer": self.layer,
            "foreign": self.foreign,
            "ok": self.ok,
            "ms": self.ms,
            "ime": self.ime,
        }


class W:
    """A `precheck.Node` plus the attributes T9's Node does not keep (§9.4)."""

    __slots__ = ("n", "a", "dup_of", "named")

    def __init__(self, node, attrs: dict[str, str]):
        self.n = node
        self.a = attrs
        self.dup_of: str | None = None
        # The accessible name, resolved by `Dump._parse` (see `resolve_names`).
        self.named = (node.text or node.desc or "").strip()

    # --- the words -------------------------------------------------------

    @property
    def word(self) -> str:
        return self.named

    @property
    def rid(self) -> str:
        return self.a.get("resource-id", "")

    @property
    def cls(self) -> str:
        return self.n.cls or ""

    @property
    def checkable(self) -> bool:
        return self.a.get("checkable") == "true"

    @property
    def checked(self) -> bool:
        return self.a.get("checked") == "true"

    @property
    def selected(self) -> bool:
        return self.a.get("selected") == "true"

    @property
    def enabled(self) -> bool:
        return self.a.get("enabled", "true") != "false"

    @property
    def long_clickable(self) -> bool:
        return self.a.get("long-clickable") == "true"

    @property
    def inside_scroll(self) -> bool:
        parent = self.n.parent
        while parent is not None:
            if parent.scrollable:
                return True
            parent = parent.parent
        return False

    # --- the semantics ---------------------------------------------------

    @property
    def in_layer(self) -> bool:
        """A node of a sheet/dialog: its parent scrolls or is a Dialog container."""
        parent = self.n.parent
        if parent is None:
            return False
        cls = parent.cls or ""
        return parent.scrollable or "Dialog" in cls or "Popup" in cls

    @property
    def state(self) -> tuple:
        """The fingerprint unit: what changes the SEMANTICS, never geometry (APE)."""
        return (self.cls, self.n.text, self.n.desc, self.checked, self.selected, self.enabled)

    @property
    def bounds(self) -> tuple[int, int, int, int]:
        return (self.n.x1, self.n.y1, self.n.x2, self.n.y2)

    @property
    def role(self) -> str:
        if self.checkable:
            return "switch"
        leaf = self.cls.rsplit(".", 1)[-1]
        if leaf in ("EditText", "AutoCompleteTextView"):
            return "textbox"
        if leaf in ("Button", "ImageButton", "CheckBox", "Switch", "RadioButton"):
            return "button"
        if self.n.clickable:
            return "button"
        return "node"

    @property
    def signature(self) -> str:
        """Stable identity: class | resource-id | text | content-desc (§9.4).

        A control with no text/desc of its own carries its label on a child (a
        Compose tab), so the resolved name is part of the identity too — without
        it, four tabs are one control (§9.4's "one identity per control").
        """
        rid = VOLATILE_ID.sub("#", self.rid)
        text = COUNTER.sub("· #", self.n.text or "")
        desc = COUNTER.sub("· #", self.n.desc or "")
        parts = [self.cls, rid, text, desc]
        if not (text or desc):
            parts.append(COUNTER.sub("· #", self.named))
        return short_sha("|".join(parts))[:16]

    @property
    def label(self) -> str:
        return f'{self.role} « {(self.word or self.rid or self.cls)[:60]} »'


def dump_screen(adb: Adb) -> Dump:
    """`dump_ui`, reimplemented: 5 attempts, the duration of ALL of them measured."""
    started = time.monotonic()
    for _ in range(5):
        done = adb.run("shell", "uiautomator", "dump", "/sdcard/nz-ui.xml", timeout=20)
        if done is None or done.returncode != 0:
            continue
        cat = adb.run("shell", "cat", "/sdcard/nz-ui.xml", timeout=20)
        text = (cat.stdout or "") if cat else ""
        if text.strip():
            return Dump(text, int((time.monotonic() - started) * 1000))
    return Dump(None, int((time.monotonic() - started) * 1000))


# --------------------------------------------------------------------------
# §9.5: naming a screen from the tree
# --------------------------------------------------------------------------

def screen_name(dump: Dump) -> str:
    for wrap in dump.app_wraps:
        desc = (wrap.n.desc or "").strip()
        if desc.endswith(" Neezer"):
            return desc
    top = dump.height * 0.20
    candidates = [w for w in dump.app_wraps if w.n.text.strip() and w.n.visible and w.n.y1 <= top]
    if candidates:
        return min(candidates, key=lambda w: (w.n.y1, w.n.x1)).n.text.strip()
    return "écran-" + dump.hash


# --------------------------------------------------------------------------
# §9.4: what an element is, and the sibling rule
# --------------------------------------------------------------------------

def named_descendants(dump: Dump, wrap: W, depth: int = 3) -> bool:
    frontier = dump.kids.get(id(wrap.n), [])
    for _ in range(depth):
        if not frontier:
            return False
        if any(kid.word for kid in frontier):
            return True
        frontier = [grand for kid in frontier for grand in dump.kids.get(id(kid.n), [])]
    return any(kid.word for kid in frontier)


def has_clickable_below(dump: Dump, wrap: W, depth: int = 4) -> bool:
    frontier = dump.kids.get(id(wrap.n), [])
    for _ in range(depth):
        if not frontier:
            return False
        if any(kid.n.clickable or kid.checkable for kid in frontier):
            return True
        frontier = [grand for kid in frontier for grand in dump.kids.get(id(kid.n), [])]
    return any(kid.n.clickable or kid.checkable for kid in frontier)


def is_element(dump: Dump, wrap: W) -> bool:
    node = wrap.n
    if node.package != PKG or not node.visible:
        return False
    if wrap.checkable or wrap.long_clickable or node.clickable:
        return True
    # A scroll container WITH controls is the page or the list, not a row: its
    # children are the elements, and tapping it does nothing a person asked for
    # (measured on the first fake-tree run: the home's own scroll root became a
    # `nothing` row on every screen). The clause exists for a list row that is
    # not clickable itself (§9.4).
    if node.scrollable and named_descendants(dump, wrap) and not has_clickable_below(dump, wrap):
        return True
    if wrap.cls.rsplit(".", 1)[-1] in TEXT_CLASSES and wrap.word and wrap.inside_scroll:
        return True
    return False


def clickable_ancestor(dump: Dump, wrap: W) -> bool:
    """Is this node inside a row that is itself an element? Then the row is the control.

    A `checkable` ancestor counts (measured on the real home, 26/11: the label
    « Libre » was a second, clickable-looking row of the switch that owns it).
    """
    parent = wrap.n.parent
    while parent is not None:
        holder = dump.wrap_of(parent)
        if parent.package == PKG and (parent.clickable or (holder is not None and holder.checkable)):
            return True
        parent = parent.parent
    return False


def elements(dump: Dump) -> tuple[list[W], int]:
    """(kept elements in walk order, number suppressed as duplicates)."""
    raw = [w for w in dump.app_wraps if is_element(dump, w)]
    suppressed = 0
    # A text line inside a clickable row is the row's label, not a second
    # control: clicking it would be the same tap, measured twice.
    candidates = []
    for wrap in raw:
        if clickable_ancestor(dump, wrap):
            suppressed += 1
            continue
        candidates.append(wrap)
    # Identity next: two nodes with one signature are one control (§9.4).
    seen: set[str] = set()
    unique: list[W] = []
    for wrap in candidates:
        sig = wrap.signature
        if sig in seen:
            suppressed += 1
            continue
        seen.add(sig)
        unique.append(wrap)
    # §4.4 the sibling rule: in one parent, ≥ 6 siblings of the same class and
    # the same child-role set → click the first 3, the rest are `duplicate`. The
    # `checkable` flag is part of the shape: without it the home's primary CTA
    # (« Lancer le Flow », measured 26/11) grouped with the six energy cards and
    # was dropped as a duplicate of a control it is not.
    groups: dict[tuple, list[W]] = defaultdict(list)
    for wrap in unique:
        child_roles = (wrap.checkable,
                       tuple(sorted({kid.cls for kid in dump.kids.get(id(wrap.n), [])})[:8]))
        groups[(id(wrap.n.parent), wrap.cls, child_roles)].append(wrap)
    for members in groups.values():
        if len(members) < 6:
            continue
        members.sort(key=lambda w: (w.n.y1, w.n.x1))
        first = members[0].signature
        for wrap in members[3:]:
            wrap.dup_of = first
            suppressed += 1
    kept = sorted((w for w in unique if w.dup_of is None), key=lambda w: (w.n.y1, w.n.x1))
    return kept, suppressed


def find_element(dump: Dump, signature: str) -> W | None:
    for wrap in dump.app_wraps:
        if wrap.signature == signature and is_element(dump, wrap):
            return wrap
    return None


# --------------------------------------------------------------------------
# §4.1: the verdicts, in the plan's priority order
# --------------------------------------------------------------------------

def classify(before: dict, after: dict, crash: list[str], pid: str,
             forced: tuple | None = None,
             baseline_ms: int = 0) -> tuple[str, str, str | None]:
    """(outcome, detail, confirm) for one transition, first match wins.

    `baseline_ms` is how long a dump took on the SAME screen before the tap. A
    freeze is a dump that is slow *for this device*, not one that crosses a
    number a faster machine was measured on: run 45907216650 produced 18
    `freeze` blockers while every dump on this emulator takes ~4 s (SwiftShader),
    so the plan's 3 000 ms read as « the app froze » 18 times on controls that
    answered normally. The floor stays 3 000 ms; a dump must also be 2.5× slower
    than the screen's own baseline to count.
    """
    if forced is not None:
        return forced[0], forced[1], forced[2] if len(forced) > 2 else None
    if crash or not pid:
        first = crash[0].strip() if crash else "le processus n'existe plus (pidof vide)"
        return "exception", first, None
    if before["textual"] >= 5 and after["ok"] and after["name"] == before["name"] and after["textual"] == 0:
        return "blank", "l'écran est vide après le tap (0 nœud nommé)", None
    if not after["ok"]:
        return "freeze", "le dump a épuisé ses 5 tentatives", None
    freeze_at = max(FREEZE_MS, int(2.5 * baseline_ms))
    if after["ms"] > freeze_at:
        return ("freeze", f"le dump a pris {after['ms']} ms (> {freeze_at}, "
                          f"base {baseline_ms} ms sur cet écran)", None)
    if after.get("anr"):
        return "console-error", after["anr"], None
    if after["name"] != before["name"]:
        return "navigated", f"{before['name']} → {after['name']}", None
    # §4.1 native: a window the previous read did not name IS a layer, even when
    # the tree is unchanged. Measured on run 45915988351: « Ouvrir le lecteur
    # plein écran » read `nothing` on three screens (a false `bloquant` on a verb)
    # because the full-screen player is a window the accessibility tree of the
    # activity did not grow into between two dumps.
    fresh_windows = sorted((Counter(after.get("windows", ()))
                            - Counter(before.get("windows", ()))).elements())
    if fresh_windows:
        return ("opened-layer", f"une fenêtre de plus : {', '.join(fresh_windows[:3])}", None)
    extra = len(after["layer"]) - len(before["layer"])
    appeared = set(after.get("foreign", ())) - set(before.get("foreign", ()))
    if appeared:
        return "opened-layer", f"une fenêtre d'une autre app : {', '.join(sorted(appeared))}", None
    if extra >= 3:
        return "opened-layer", "une couche s'est ouverte", None
    if after["ime"] and not before["ime"]:
        return "opened-layer", "le clavier s'est ouvert", None
    if after["nodes"] != before["nodes"]:
        return "state-changed", first_difference(before["nodes"], after["nodes"]), None
    return "nothing", "aucun changement en 3 000 ms (2 dumps)", None


def first_difference(before: tuple, after: tuple) -> str:
    for left, right in zip(before, after):
        if left != right:
            return (f"{left[0].rsplit('.', 1)[-1]} « {left[1] or left[2]} » → "
                    f"« {right[1] or right[2]} »")
    if len(before) != len(after):
        return f"{len(before)} nœuds → {len(after)} nœuds"
    return "empreinte différente"


# --------------------------------------------------------------------------
# §4.5: destructive actions, decided once
# --------------------------------------------------------------------------

def destructive_of(name: str) -> tuple[str, bool] | None:
    word = (name or "").strip()
    if word in DESTRUCTIVE:
        return DESTRUCTIVE[word]
    for label, regime in DESTRUCTIVE.items():
        if word.startswith(label):
            return regime
    return None


def confirm_present(dump: Dump) -> str | None:
    for wrap in dump.app_wraps:
        if not wrap.n.visible:
            continue
        if wrap.word in CONFIRM_WORDS:
            return wrap.word
        if "Dialog" in wrap.cls or "Popup" in wrap.cls:
            return wrap.cls.rsplit(".", 1)[-1]
    return None


# --------------------------------------------------------------------------
# the walk
# --------------------------------------------------------------------------

class Screen:
    __slots__ = ("index", "name", "hash", "path", "elements", "dup", "restore")

    def __init__(self, index: int, name: str, digest: str,
                 path: list[tuple[int, int]], elements: int = 0, dup: int = 0):
        self.index = index
        self.name = name
        self.hash = digest
        self.path = path
        self.elements = elements
        self.dup = dup
        self.restore = "ok"


class Walker:
    def __init__(self, adb: Adb, out: Path, max_nodes: int, max_minutes: int):
        self.adb = adb
        self.out = out
        self.max_nodes = max_nodes
        self.deadline = time.monotonic() + max_minutes * 60
        self.screens: list[Screen] = []
        self.by_name: dict[str, Screen] = {}
        self.rows: list[dict] = []
        self.clicked = 0
        self.suppressed = 0
        self.dpi = 420
        self.stopped: str | None = None
        self.current: Screen | None = None
        self.label_collision = 0
        self.shots = 0
        self.anomalies = 0
        self.capped = 0
        # §9.6: the tree read for this element IS the « avant » of the next one
        # while the screen has not changed. Measured on run 45915988351: three
        # dumps per element (`resolve`'s, the post-tap one, the `nothing`
        # confirmation) at ~2.5 s each on SwiftShader, 14 s per click, and the
        # pass stopped at `coverage=78.1%` with `stopped=budget`.
        self.tree: Dump | None = None
        self.tree_screen: Screen | None = None
        # §4.1: an exception is attributed "depuis le dernier repère" — the crash
        # buffer keeps its record, so a second read without a mark would count
        # the same crash once per tap (measured on the fake tree: 6 exceptions
        # for one injected FATAL EXCEPTION).
        self.crash_mark = "\n".join(self.adb.crash_lines())

    # --- the device ------------------------------------------------------

    def dump(self) -> Dump:
        return dump_screen(self.adb)

    def remember(self, screen: Screen, dump: Dump) -> None:
        """Keep `dump` as the screen's current tree, or drop the cache."""
        self.tree = dump if (dump is not None and dump.ok) else None
        self.tree_screen = screen if self.tree is not None else None

    def forget(self) -> None:
        self.tree = None
        self.tree_screen = None

    def settle(self, dump: Dump, tries: int = 6) -> Dump:
        """Wait while the screen still says it is loading.

        Measured on run 45907216650: two screens reached mid-load were named
        « Chargement… » (and one by a paragraph of copy), then could not be
        restored — 2 of its 4 `restore=failed` and ~3 min of the pass's budget.
        A screen with fewer than 5 names is treated the same way.
        """
        for _ in range(tries):
            loading = any(word.startswith("Chargement") for word in dump.texts)
            if not loading and dump.textual >= 5:
                return dump
            time.sleep(1.2)
            dump = self.dump()
        return dump

    def start_app(self) -> bool:
        component = self.adb.launcher()
        if not component:
            return False
        self.forget()
        self.adb.shell("am", "force-stop", PKG, timeout=15)
        self.adb.shell("am", "start", "-W", "-n", component, timeout=45)
        for _ in range(20):
            dump = self.dump()
            if dump.ok and dump.textual >= 3:
                return True
        return False

    def same_screen(self, dump: Dump, screen: Screen) -> bool:
        """Is `dump` the screen `screen` names?

        A renamed collision carries its hash in the label (§9.5:
        « Recherche Neezer · 5f23cd6a ») while a dump only knows the plain title,
        so comparing the two names alone made every collided screen unrestorable
        — measured on run 45898606608: 5 screens `restore=failed`, 36 elements
        `unreachable`, coverage 76.5%. The title AND the hash must agree.
        """
        if dump.name == screen.name:
            return True
        base, _, digest = screen.name.partition(" · ")
        if dump.name != base:
            return False
        return bool(digest) and dump.hash == digest

    def goto(self, screen: Screen) -> bool:
        if self.current is screen:
            return True
        for _ in range(2):
            if not self.start_app():
                return False
            for (x, y) in screen.path:
                self.adb.tap(x, y)
                self.dump()
            dump = self.dump()
            if dump.ok and self.same_screen(dump, screen):
                self.current = screen
                self.remember(screen, dump)
                return True
        return False

    # --- evidence --------------------------------------------------------

    def screen_dir(self) -> Path:
        path = self.out / "screens"
        path.mkdir(parents=True, exist_ok=True)
        return path

    def write_screen_files(self, screen: Screen, dump: Dump) -> None:
        directory = self.screen_dir()
        stem = f"{screen.index:02d}-{slug(screen.name)}"
        directory.joinpath(f"{stem}.xml").write_text(dump.text or "", encoding="utf-8")
        if self.adb.screencap(directory / f"{stem}.png"):
            self.shots += 1

    def shot(self, screen: Screen, wrap: W, outcome: str) -> str:
        rank = sum(1 for row in self.rows if row["screen"]["name"] == screen.name) + 1
        name = (f"{screen.index:02d}-{slug(screen.name)}-{rank}-"
                f"{slug(wrap.word or wrap.rid or wrap.cls)}-{outcome}.png")
        self.adb.screencap(self.screen_dir() / name)
        return f"screens/{name}"

    # --- one element -----------------------------------------------------

    def tap_point(self, wrap: W) -> tuple[int, int]:
        x1, y1, x2, y2 = wrap.bounds
        cap = int(24 * self.dpi / 160.0)
        return (x1 + x2) // 2, y1 + min(max((y2 - y1) // 2, 1), cap)

    def resolve(self, screen: Screen, signature: str) -> tuple[Dump, W] | None:
        """The element in the current tree, scrolled into view at most 3 times.

        The tree already read for this screen is tried FIRST: a click that left
        the screen in place (every `state-changed` and `nothing`) makes it the
        exact « avant » state §9.6 asks for, and reading it again costs ~2.5 s on
        the SwiftShader emulator for the same bytes.
        """
        if self.tree is not None and self.tree_screen is screen:
            cached = find_element(self.tree, signature)
            if (cached is not None and cached.n.visible
                    and 0 <= cached.n.y1 and cached.n.y2 <= self.tree.height):
                return self.tree, cached
        for attempt in range(4):
            dump = self.dump()
            found = find_element(dump, signature)
            if found is not None and found.n.visible and 0 <= found.n.y1 and found.n.y2 <= dump.height:
                return dump, found
            if attempt < 3:
                self.adb.swipe_up(dump.width, dump.height)
        return None

    def handle(self, screen: Screen, signature: str) -> None:
        if time.monotonic() > self.deadline:
            self.stopped = "budget"
            return
        if self.clicked >= self.max_nodes:
            self.stopped = "nodes"
            return
        if not self.goto(screen):
            screen.restore = "failed"
            return
        if self.tree_screen is not screen:
            self.forget()
        resolved = self.resolve(screen, signature)
        if resolved is None:
            self.row(screen, signature, "", "unreachable",
                     "ni visible ni atteint par 3 glissements", 0, None, None, "node")
            return
        before_dump, wrap = resolved
        before = before_dump.view()
        before_windows = self.adb.windows()
        before["windows"] = before_windows
        regime = destructive_of(wrap.word)
        if regime is not None and regime[0] == "fin":
            self.row(screen, signature, wrap.word, "skipped-session",
                     "action irréversible : jouée par --scenario session", 0, None, None, wrap.label)
            return
        x, y = self.tap_point(wrap)
        started = time.monotonic()
        self.adb.tap(x, y)
        after_dump = self.dump()
        ms = int((time.monotonic() - started) * 1000)
        after_windows = self.adb.windows()
        crash = self.adb.crash_lines()
        joined = "\n".join(crash)
        fresh = crash if joined != self.crash_mark else []
        if fresh:
            self.crash_mark = joined
        pid = self.adb.pid()
        after = after_dump.view()
        after["windows"] = after_windows
        if after_dump.ms > ANR_CHECK_MS or fresh or not pid:
            lines = self.adb.anr()
            if lines:
                after["anr"] = lines[0].strip()
        confirm = None
        if regime is not None and regime[0] in ("passe", "jamais"):
            hit = confirm_present(after_dump)
            outcome = "blocked-destructive"
            detail = ("clic arrêté avant la confirmation" if regime[0] == "passe"
                      else "clic arrêté à la feuille")
            confirm = "confirm-reached" if hit else "no-confirm"
            if hit and regime[0] == "passe":
                self.cancel_destructive(screen, before)
            elif hit:
                self.adb.key("BACK")
                self.dump()
            final_dump = after_dump
        else:
            outcome, detail, _ = classify(before, after, fresh, pid, forced=None,
                                          baseline_ms=before_dump.ms)
            if outcome == "nothing":
                time.sleep(NOTHING_WAIT_S)
                final_dump = self.dump()
                confirmed = final_dump.view()
                confirmed["windows"] = self.adb.windows()
                outcome, detail, _ = classify(before, confirmed, fresh, pid,
                                              baseline_ms=before_dump.ms)
                ms = int((time.monotonic() - started) * 1000)
                after = confirmed
                if outcome == "nothing":
                    # The window count is the other half of the verdict: a
                    # `nothing` where no window appeared either is a door that
                    # really did nothing, not a layer the tree cannot see.
                    detail = (f"aucun changement sur 2 dumps en {ms} ms "
                              f"(fenêtres {len(confirmed.get('windows', ()))})")
            else:
                final_dump = after_dump
        self.clicked += 1
        # The tree that is on screen NOW is the next element's « avant » (§9.6):
        # keep it while the screen stayed in place, throw it away the moment it
        # did not (a navigation, a layer, a blank, a crash).
        if outcome in ("state-changed", "nothing") and after["name"] == before["name"]:
            self.remember(screen, final_dump)
        else:
            self.forget()
        evidence = None
        if outcome in ("exception", "blank", "freeze", "nothing", "console-error"):
            if self.anomalies < ANOMALY_SHOTS:
                self.anomalies += 1
                evidence = self.shot(screen, wrap, outcome)
            else:
                self.capped += 1
        row = self.row(screen, signature, wrap.word, outcome, detail, ms, evidence,
                       confirm, wrap.label)
        row["reversible"] = bool(regime and regime[1])
        if outcome not in ("exception", "blank", "freeze"):
            self.restore(screen, before, after, outcome)
        if outcome == "navigated":
            self.discover(after, screen.path + [(x, y)])

    def cancel_destructive(self, screen: Screen, before: dict) -> None:
        """Cancel, then check the fingerprint came back (§4.5)."""
        for word in ("Annuler", "Non"):
            dump = self.dump()
            target = next((w for w in dump.app_wraps if w.word == word and w.n.visible), None)
            if target is not None:
                self.adb.tap(*self.tap_point(target))
                break
        else:
            self.adb.key("BACK")
        after = self.dump().view()
        if after["nodes"] != before["nodes"] and after["name"] == before["name"]:
            self.rows.append({
                "app": APP,
                "screen": {"name": screen.name, "path": f"tree:{screen.hash}"},
                "element": {"role": "screen", "name": "annulation destructive",
                            "selector": None, "via": "tree",
                            "signature": short_sha("annulation"), "duplicate_of": None},
                "action": "click",
                "outcome": "state-changed",
                "detail": "l'empreinte d'avant n'est pas revenue après l'annulation",
                "ms": 0,
                "severity": "sérieux",
                "bug_id": None,
                "evidence": None,
                "confirm": None,
            })

    def restore(self, screen: Screen, before: dict, after: dict, outcome: str) -> None:
        if outcome in ("navigated", "opened-layer") or after.get("ime"):
            for _ in range(3):
                self.adb.key("BACK")
                dump = self.dump()
                view = dump.view()
                if view["name"] == before["name"]:
                    self.remember(screen, dump)
                    return
            self.current = None
            self.forget()
        elif after["name"] != before["name"]:
            self.current = None
            self.forget()

    # A screen a person can come back to has a NAME. A loading placeholder and
    # the proxy's own 503 notice are states, not screens: the pass discovered
    # them at run 45929355454 (three of them, 0 element each) and paid the full
    # restore cycle — `am force-stop` + the tap path — to re-open nothing.
    TRANSIENT = ("Chargement", "Deezer proxy disabled")

    def discover(self, after: dict, path: list[tuple[int, int]]) -> None:
        name = after["name"]
        if name.startswith(self.TRANSIENT):
            return
        existing = self.by_name.get(name)
        if existing is not None:
            if existing.hash != after["hash"] and "·" not in existing.name:
                # §9.5: two different screens under one label are separated by
                # the hash of their first 20 texts; the label is reported.
                suffix = f"{name} · {after['hash']}"
                if suffix in self.by_name:
                    return
                existing.name = f"{name} · {existing.hash}"
                self.by_name[existing.name] = existing
                screen = Screen(len(self.screens) + 1, suffix, after["hash"], list(path))
                self.by_name[suffix] = screen
                self.screens.append(screen)
                self.label_collision += 1
            return
        screen = Screen(len(self.screens) + 1, name, after["hash"], list(path))
        self.by_name[name] = screen
        self.screens.append(screen)

    def row(self, screen: Screen, signature: str, name: str, outcome: str, detail: str,
            ms: int, evidence: str | None, confirm: str | None, label: str,
            action: str = "click") -> dict:
        severity = None
        if outcome == "blocked-destructive":
            severity = "bloquant" if confirm == "no-confirm" else None
        elif outcome in ("exception", "blank", "freeze"):
            severity = "bloquant"
        elif outcome == "nothing":
            severity = "bloquant" if starts_with_verb(name) else "mineur"
        elif outcome == "console-error":
            severity = "sérieux"
        elif outcome == "unreachable":
            severity = "sérieux" if starts_with_verb(name) else "mineur"
        row = {
            "app": APP,
            "screen": {"name": screen.name, "path": f"tree:{screen.hash}"},
            "element": {
                "role": label.split(" ", 1)[0],
                "name": name,
                "selector": None,
                "via": "tree",
                "signature": signature,
                "duplicate_of": None,
            },
            "action": action,
            "outcome": outcome,
            "detail": detail,
            "ms": ms,
            "severity": severity,
            "bug_id": None,
            "evidence": evidence,
            "confirm": confirm,
        }
        self.rows.append(row)
        return row


# --------------------------------------------------------------------------
# §9.6: the pass
# --------------------------------------------------------------------------

def run_pass(walker: Walker, only: str | None) -> None:
    if only:
        target = next((screen for screen in walker.screens if screen.name == only), None)
        if target is None:
            print(f"bughunt: écran inconnu « {only} » (lancer une passe complète d'abord)",
                  file=sys.stderr)
            raise SystemExit(2)
        explore_screen(walker, target)
        return
    if not walker.start_app():
        raise SystemExit("bughunt: l'app ne peint pas son écran d'accueil après le lancement")
    first = walker.dump()
    if not first.ok:
        raise SystemExit("bughunt: aucun arbre d'accessibilité au lancement")
    walker.by_name[first.name] = Screen(1, first.name, first.hash, [])
    walker.screens.append(walker.by_name[first.name])
    pending = list(walker.screens)
    done: set[str] = set()
    while pending and not walker.stopped:
        if time.monotonic() > walker.deadline:
            walker.stopped = "budget"
            break
        screen = pending.pop(0)
        if screen.name in done:
            continue
        done.add(screen.name)
        explore_screen(walker, screen)
        for discovered in walker.screens:
            if discovered.name not in done and all(item is not discovered for item in pending):
                pending.append(discovered)


def explore_screen(walker: Walker, screen: Screen) -> None:
    if not walker.goto(screen):
        screen.restore = "failed"
        print(f"bughunt: écran « {screen.name} » non restauré (chemin perdu)", file=sys.stderr)
        return
    # `goto` just verified this screen with a dump: read it again only if the
    # cache is empty (`--only` on a lost path, a screen reached by `start_app`).
    if walker.tree is not None and walker.tree_screen is screen:
        dump = walker.tree
    else:
        dump = walker.dump()
    if not dump.ok:
        screen.restore = "failed"
        return
    dump = walker.settle(dump)
    walker.remember(screen, dump)
    walker.write_screen_files(screen, dump)
    kept, suppressed = elements(dump)
    screen.elements = len(kept)
    screen.dup = suppressed
    walker.suppressed += suppressed
    for wrap in kept:
        if walker.stopped:
            break
        walker.handle(screen, wrap.signature)
        if walker.current is None:
            walker.goto(screen)


# --------------------------------------------------------------------------
# §9.8: the three scenarios
# --------------------------------------------------------------------------

def media_state(adb: Adb) -> str:
    for line in adb.shell("dumpsys", "media_session", timeout=20).splitlines():
        if "state=PlaybackState" in line:
            return line.strip()
    return ""


# Android prints the state as `state=BUFFERING(6)`, never as `state=6`: run
# 45924564740 measured a « Suivant » burst on a player that WAS playing
# (`Background started FGS: NzMediaService` at +9 s) and still read `playing=0`,
# because the test was a literal `"state=3" in …`. One regex, both shapes.
PLAYBACK = re.compile(r"state=(PLAYING|PAUSED|BUFFERING|STOPPED|ERROR|ENDED|NONE|IDLE)\b")


def playback_state(adb: Adb) -> str:
    """The media session's state word: PLAYING, BUFFERING, PAUSED…"""
    match = PLAYBACK.search(media_state(adb))
    return match.group(1) if match else ""


def crash_mark(adb: Adb) -> str:
    """What the crash buffer already held: only what comes AFTER is ours."""
    return "\n".join(adb.crash_lines())


def fresh_crash(adb: Adb, mark: str) -> tuple[list[str], str]:
    """(new crash lines, current mark) — an earlier run's crash is not this one."""
    lines = adb.crash_lines()
    joined = "\n".join(lines)
    return (lines if joined != mark else []), joined


def media_position(adb: Adb) -> int | None:
    match = re.search(r"position=(-?\d+)", media_state(adb))
    return int(match.group(1)) if match else None


def media_title(adb: Adb) -> str:
    out = adb.shell("dumpsys", "media_session", timeout=20)
    match = re.search(r"description=([^,}\n]+)", out)
    if match:
        return match.group(1).strip()
    match = re.search(r"metadata:[^\n]*", out)
    return match.group(0) if match else ""


def open_settings(walker: Walker) -> bool:
    """Réglages, proven by the screen itself — never by a tap that did nothing.

    Measured on run 45915988351: the previous version returned True as soon as
    the post-tap dump was readable, so the `settings` and `session` scenarios
    walked the screen they were already on (`controls=0`, « Déconnexion »
    introuvable) instead of failing on the door they never reached.
    """
    for _ in range(3):
        dump = walker.dump()
        if is_settings(dump):
            return True
        door = next((w for w in dump.app_wraps
                     if w.word == "Réglages" and w.n.visible and w.n.clickable), None)
        if door is not None:
            walker.adb.tap(*walker.tap_point(door))
            continue
        walker.adb.key("BACK")
    return is_settings(walker.dump())


def is_settings(dump: Dump) -> bool:
    """The settings page names its own search field (measured, run 45915988351)."""
    return dump.ok and any(w.word.startswith("Chercher un réglage") for w in dump.app_wraps)


# The shell's own controls, never a setting: the four tabs, the mini-bar's door
# and its play button. A scenario that taps one of them measures navigation.
SHELL = ("Accueil", "Rechercher", "Biblio", "Podcasts", "Réglages",
         "Ouvrir le lecteur plein écran", "Lecture")


def settings_sections(dump: Dump) -> list[W]:
    """The named rows of the Réglages page that open a section."""
    return [w for w in dump.app_wraps
            if w.n.visible and w.n.clickable and w.word and w.word not in SHELL
            and not w.word.startswith("Chercher")]


def door_of(walker: Walker) -> "W | None":
    """The mini-bar's « Ouvrir le lecteur plein écran », when it is on screen."""
    dump = walker.dump()
    return next((w for w in dump.app_wraps
                 if (w.n.desc or "").strip() == "Ouvrir le lecteur plein écran"
                 and w.n.visible), None)


def is_playing(adb: Adb) -> bool:
    """PLAYING, not « a mini-player exists ».

    Two measured lessons: run 45915988351 had `door_of` find the mini-bar's door
    on an IDLE player and call it `deja-en-lecture`; run 45924564740 then found
    the reverse — a real playback (`NzMediaService` foreground, `BUFFERING` then
    `PLAYING`) read as `playing=0` because the test looked for the literal
    `state=3`. `playback_state` reads what Android prints.
    """
    return playback_state(adb) == "PLAYING"


def start_playback(walker: Walker) -> tuple["W | None", str]:
    """Make a track play, the way a person would; returns (door, what was tried).

    §9.8 says « ouvrir le lecteur, démarrer la lecture ». On a fresh install
    nothing is playing, and the home offers no « Lire » (measured on the first
    run, 26/11: `aucun « Ouvrir le lecteur plein écran » ni « Lire » sur
    l'accueil`) — the door is then its own « Lancer le Flow », or the first row
    of the library. Every attempt is named, so a FAIL says WHICH door was
    missing instead of a sentence that hides it.
    """
    adb = walker.adb
    tried: list[str] = []
    door = door_of(walker)
    if door is not None and is_playing(adb):
        return door, "deja-en-lecture"
    for label in ("Lire", "Lancer le Flow"):
        dump = walker.dump()
        target = next((w for w in dump.app_wraps if w.word.startswith(label) and w.n.visible), None)
        if target is None:
            continue
        tried.append(label)
        adb.tap(*walker.tap_point(target))
        for _ in range(24):
            if is_playing(adb):
                break
            time.sleep(0.5)
        if is_playing(adb):
            door = door_of(walker)
            if door is not None:
                return door, tried[-1]
    dump = walker.dump()
    tab = next((w for w in dump.app_wraps if w.n.clickable and w.n.visible
                and "biblio" in w.word.lower()), None)
    if tab is not None:
        tried.append(tab.word)
        adb.tap(*walker.tap_point(tab))
        for _ in range(6):
            dump = walker.dump()
            row = next((w for w in dump.app_wraps if w.n.clickable and w.n.visible and w.word
                        and w is not tab and w.n.y1 > dump.height * 0.15), None)
            if row is None:
                adb.swipe_up(dump.width, dump.height)
                continue
            tried.append(f"ligne:{row.word[:24]}")
            adb.tap(*walker.tap_point(row))
            for _ in range(24):
                if is_playing(adb):
                    break
                time.sleep(0.5)
            if is_playing(adb):
                door = door_of(walker)
                if door is not None:
                    return door, tried[-1]
            break
    return None, "|".join(tried) or "aucune-porte"


def scenario_burst(walker: Walker) -> dict:
    """20 « Suivant » at 100 ms, then 20 alternating — the owner's first symptom."""
    adb = walker.adb
    result = {"name": SCENARIO_LABEL["burst"], "verdict": "FAIL", "numbers": "", "evidence": ""}
    started = time.monotonic()
    mark = crash_mark(adb)

    if not walker.start_app():
        result["numbers"] = "l'app ne peint pas son accueil"
        return result
    door, tried = start_playback(walker)
    if door is None:
        result["numbers"] = f"aucune lecture n'a démarré (portes essayées : {tried})"
        return result
    result["started_by"] = tried
    adb.tap(*walker.tap_point(door))
    # The full-screen player is a window, and on SwiftShader it can paint later
    # than one dump: wait on its own marker, the way the web harness waits on a
    # screen's marker, instead of reading one dump as « it did nothing ».
    next_node = prev_node = None
    dump = walker.dump()
    for _ in range(5):
        next_node = next((w for w in dump.app_wraps if w.word == "Suivant" and w.n.visible), None)
        if next_node is not None:
            break
        time.sleep(0.8)
        dump = walker.dump()
    consequence = "plein-écran" if next_node is not None else f"écran « {dump.name} »"
    if next_node is None:
        result["numbers"] = f"le lecteur plein écran ne peint pas de « Suivant » ({consequence})"
        return result
    play = next((w for w in dump.app_wraps if w.word == "Lecture" and w.n.visible), None)
    if play is not None:
        adb.tap(*walker.tap_point(play))
        dump = walker.dump()
    prev_node = next((w for w in dump.app_wraps if w.word == "Précédent" and w.n.visible), None)
    result["player"] = consequence
    next_point = walker.tap_point(next_node)
    prev_point = walker.tap_point(prev_node) if prev_node is not None else next_point
    title_before = media_title(adb)
    for _ in range(20):
        adb.tap(*next_point)
        time.sleep(0.100)
    walker.dump()
    title_after = media_title(adb)
    adb.tap(*next_point)
    walker.dump()
    title_one = media_title(adb)
    # 40 taps at 100 ms restart the stream 40 times: the player is BUFFERING at
    # the last one on this bench (measured, run 45929355454: `playing=0` with
    # `title_changed=1`). The owner's symptom is a track that never starts, not
    # one that starts 2 s later — so wait for PLAYING, then measure the clock.
    for _ in range(30):
        if playback_state(adb) == "PLAYING":
            break
        time.sleep(0.5)
    playing = 1 if playback_state(adb) == "PLAYING" else 0
    clock_a = media_position(adb)
    time.sleep(1.5)
    clock_b = media_position(adb)
    for index in range(20):
        adb.tap(*(next_point if index % 2 == 0 else prev_point))
        time.sleep(0.100)
    walker.dump()
    crash, _ = fresh_crash(adb, mark)
    alive = 1 if adb.pid() else 0
    changed = 1 if title_after and title_before and title_after != title_before else 0
    changed_again = 1 if title_one and title_after and title_one != title_after else 0
    advanced = 1 if (clock_a is not None and clock_b is not None and clock_b > clock_a) else 0
    playing_end = 1 if playback_state(adb) == "PLAYING" else 0
    seconds = int(time.monotonic() - started)
    ok = bool(changed and changed_again and alive and not crash and advanced)
    result["verdict"] = "PASS" if ok else "FAIL"
    result["numbers"] = (f"taps=40 gap_ms=100 playing={playing} title_changed={changed} "
                         f"changed_again={changed_again} clock_delta={'yes' if advanced else 'no'} "
                         f"playing_end={playing_end} crash={len(crash)} seconds={seconds}")
    result["line"] = (f"skip_burst={'ok' if ok else 'fail'} taps=40 gap_ms=100 playing={playing} "
                      f"title_changed={changed} crash={len(crash)}")
    return result


def scenario_settings(walker: Walker) -> dict:
    """Apply the controls of Réglages, section by section (§9.8).

    The native tree does NOT mark a Compose switch as `checkable` (measured on
    run 45915988351: `checkable=0` on ten of the eleven dumps, the home's six
    energy cards being the only ones), so the previous version counted zero
    controls on every screen and reported `controls=0` as if the settings did
    nothing. The scenario now opens each section the Réglages page exposes,
    taps its named controls one by one, and counts the ones applied IN PLACE —
    a tap that leaves the section is a door, counted apart. A blank or a crash
    is the failure, never the absence of a `checkable` attribute.
    """
    adb = walker.adb
    result = {"name": SCENARIO_LABEL["settings"], "verdict": "FAIL", "numbers": "", "evidence": ""}
    started = time.monotonic()
    mark = crash_mark(adb)
    if not walker.start_app() or not open_settings(walker):
        result["numbers"] = "« Réglages » introuvable"
        return result
    root = walker.dump()
    sections = [word.word for word in settings_sections(root)][:4]
    controls = 0
    doors = 0
    blanks = 0
    crash: list[str] = []
    visited: set[str] = set()
    deadline = time.monotonic() + 3 * 60
    for label in sections:
        if time.monotonic() > deadline or crash:
            break
        if not open_settings(walker):
            break
        if not open_settings_section(walker, label):
            continue
        for _ in range(6):
            if time.monotonic() > deadline:
                break
            dump = walker.dump()
            if not dump.ok:
                blanks += 1
                break
            crash, _ = fresh_crash(adb, mark)
            if crash:
                break
            targets = [w for w in dump.app_wraps
                       if w.n.visible and (w.n.clickable or w.checkable) and w.word
                       and w.signature not in visited and w.word not in SHELL
                       and not w.word.startswith("Retour")
                       and not (destructive_of(w.word) and destructive_of(w.word)[0] != "reversible")]
            if not targets:
                break
            wrap = targets[0]
            visited.add(wrap.signature)
            adb.tap(*walker.tap_point(wrap))
            after = walker.dump()
            if not after.ok or after.textual == 0:
                blanks += 1
                break
            if after.name != dump.name:
                # The control left the section (a door, or a setting that opens
                # its own screen): count it apart and come back for the next.
                doors += 1
                if not open_settings(walker) or not open_settings_section(walker, label):
                    break
                continue
            controls += 1
    seconds = int(time.monotonic() - started)
    ok = bool(controls > 0 and not crash and not blanks)
    result["verdict"] = "PASS" if ok else "FAIL"
    result["numbers"] = (f"controls={controls} doors={doors} blanks={blanks} "
                         f"crash={len(crash)} seconds={seconds}")
    result["line"] = (f"settings_apply={'ok' if ok else 'fail'} controls={controls} "
                      f"blanks={blanks} crash={len(crash)}")
    return result


def open_settings_section(walker: Walker, label: str) -> bool:
    """Open one named row of Réglages, proven by the screen that replaced it.

    The page scrolls: « Compte » (the only door to « Déconnexion », verified in
    `SettingsCopy.kt`) sits below the fold, so a lookup on the first dump only
    ever found the three sections at the top — measured on runs 45924564740 and
    45929355454, where the session scenario tapped « Sources » and never saw the
    logout button it was looking for.
    """
    for attempt in range(5):
        dump = walker.dump()
        row = next((w for w in dump.app_wraps if w.word == label and w.n.visible), None)
        if row is not None:
            walker.adb.tap(*walker.tap_point(row))
            after = walker.dump()
            return after.ok and not is_settings(after)
        if attempt < 4:
            walker.adb.swipe_up(dump.width, dump.height)
    return False


def scenario_session(walker: Walker) -> dict:
    """Log out, prove the login screen paints; `explore.sh` signs back in (§4.5)."""
    adb = walker.adb
    result = {"name": SCENARIO_LABEL["session"], "verdict": "FAIL", "numbers": "", "evidence": ""}
    started = time.monotonic()
    mark = crash_mark(adb)
    if not walker.start_app() or not open_settings(walker):
        result["numbers"] = "« Réglages » introuvable"
        return result
    # « Déconnexion » lives in the « Compte » card (`SourcesSections.kt`,
    # `AccountSection`), behind the « Compte » row of Réglages.
    if not open_settings_section(walker, "Compte"):
        result["numbers"] = "« Compte » introuvable dans Réglages"
        return result
    logout = None
    where = ""
    for attempt in range(8):
        dump = walker.dump()
        where = dump.name
        logout = next((w for w in dump.app_wraps if w.word == "Déconnexion" and w.n.visible), None)
        if logout is not None:
            break
        if attempt % 2 == 1:
            adb.swipe_up(dump.width, dump.height)
    if logout is None:
        result["numbers"] = f"« Déconnexion » introuvable sur « {where} »"
        return result
    adb.tap(*walker.tap_point(logout))
    confirm = None
    for _ in range(10):
        dump = walker.dump()
        confirm = next((w for w in dump.app_wraps if w.word == "Confirmer" and w.n.visible), None)
        if confirm is not None:
            break
    if confirm is None:
        result["numbers"] = "« Confirmer » introuvable : la déconnexion ne demande rien"
        return result
    adb.tap(*walker.tap_point(confirm))
    painted = False
    for _ in range(20):
        dump = walker.dump()
        if dump.ok and dump.textual >= 2 and any(
                w.word in ("Se connecter", "Adresse du serveur", "Ton compte")
                for w in dump.app_wraps):
            painted = True
            break
    crash, _ = fresh_crash(adb, mark)
    alive = adb.pid()
    seconds = int(time.monotonic() - started)
    ok = bool(painted and alive and not crash)
    result["verdict"] = "PASS" if ok else "FAIL"
    result["numbers"] = (f"login_painted={1 if painted else 0} crash={len(crash)} "
                         f"alive={1 if alive else 0} seconds={seconds}")
    result["line"] = (f"session_logout={'ok' if ok else 'fail'} "
                      f"login_painted={1 if painted else 0} crash={len(crash)}")
    return result


# --------------------------------------------------------------------------
# the report (§4.7, §4.8)
# --------------------------------------------------------------------------

def build_bugs(rows: list[dict]) -> list[dict]:
    merged: dict[str, dict] = {}
    for row in rows:
        if row.get("severity") is None:
            continue
        key = (f"{slug(row['screen']['name'])}|{row['outcome']}|"
               f"{slug(row['element']['name'] or 'sans-nom')}")
        if key in merged:
            merged[key]["occurrences"] += 1
            continue
        merged[key] = {
            "id": 0,
            "key": key,
            "severity": row["severity"],
            "screen": row["screen"]["name"],
            "element": f"{row['element']['role']} « {row['element']['name']} »",
            "outcome": row["outcome"],
            "occurrences": 1,
            "steps": [
                f"ouvrir {row['screen']['name']}",
                f"cliquer {row['element']['role']} « {row['element']['name']} »",
                row["detail"] or row["outcome"],
            ],
            "evidence": row.get("evidence") or "",
        }
    order = {"bloquant": 0, "sérieux": 1, "mineur": 2}
    bugs = sorted(merged.values(), key=lambda bug: (order.get(bug["severity"], 3), bug["key"]))
    for index, bug in enumerate(bugs, 1):
        bug["id"] = index
    return bugs


def summary(run: dict) -> dict:
    rows = run.get("rows", [])
    clicked = sum(1 for row in rows
                  if row["outcome"] not in ("unreachable", "skipped-session", "duplicate"))
    skipped = sum(1 for row in rows if row["outcome"] == "skipped-session")
    found = run.get("elements", 0)
    # §4.4: coverage is clicked over the elements the pass COULD click. An
    # `unreachable` stays in the denominator (honest), a `skipped-session` does
    # not: the plan says never to click it, so counting it would make the
    # harness's own rule look like a miss.
    return {
        "screens": len(run.get("screens", [])),
        "elements": found,
        "clicked": clicked,
        "coverage": percent(clicked, max(found - skipped, 0)),
        "duplicates": run.get("duplicates", 0),
        "nothing": sum(1 for row in rows if row["outcome"] == "nothing"),
        "named_nothing": sum(1 for row in rows if row["outcome"] == "nothing"
                             and starts_with_verb(row["element"]["name"])),
        "blank": sum(1 for row in rows if row["outcome"] == "blank"),
        "exceptions": sum(1 for row in rows if row["outcome"] == "exception"),
        "freezes": sum(1 for row in rows if row["outcome"] == "freeze"),
        "unreachable": sum(1 for row in rows if row["outcome"] == "unreachable"),
        "seconds": run.get("seconds", 0),
        "capped": run.get("capped", 0),
        "stopped": run.get("stopped"),
    }


def summary_line(run: dict) -> str:
    totals = summary(run)
    # §4.4: the percentage AND the raw count, in the shape the web harness
    # prints (`coverage=100.0% (13/13)`), so the judge reads one vocabulary.
    counted = max(totals["elements"] - sum(
        1 for row in run.get("rows", []) if row["outcome"] == "skipped-session"), 0)
    line = (f"bughunt: screens={totals['screens']} elements={totals['elements']} "
            f"clicked={totals['clicked']} "
            f"coverage={totals['coverage']}% ({totals['clicked']}/{counted}) "
            f"duplicates={totals['duplicates']} nothing={totals['nothing']} "
            f"named_nothing={totals['named_nothing']} "
            f"blank={totals['blank']} exceptions={totals['exceptions']} "
            f"freezes={totals['freezes']} seconds={totals['seconds']}")
    if totals.get("capped"):
        line += f" screenshots_capped={totals['capped']}"
    if totals["stopped"]:
        line += f" stopped={totals['stopped']}"
    return line


def render_md(run: dict) -> str:
    totals = summary(run)
    device = run.get("device", {})
    lines = [
        "# Chasse aux bugs — application native",
        "",
        f"`{run.get('date', '')}` · tête `{run.get('head', 'unknown')}` · "
        f"{device.get('avd', 'AVD pixel_6 API 35')} · {device.get('size', '1080×2400')}",
        f"`{summary_line(run)}`",
        "",
        "## Ce que le propriétaire a demandé d'abord",
        "",
        "| Scénario | Verdict | Nombres mesurés | Preuve |",
        "|---|---|---|---|",
    ]
    scenarios = run.get("scenarios", {})
    for name in ("burst", "burst-alternated", "settings", "session"):
        verdict = scenarios.get(name)
        if verdict is None:
            lines.append(f"| {SCENARIO_LABEL[name]} | NON JOUÉ | — | — |")
            continue
        lines.append(f"| {verdict.get('name', SCENARIO_LABEL[name])} | {verdict.get('verdict', 'NON JOUÉ')} | "
                     f"`{verdict.get('numbers', '')}` | {verdict.get('evidence') or '—'} |")
    bugs = run.get("bugs", [])
    lines += ["", "## Bugs (dédupliqués, le plus grave d'abord)", ""]
    if not bugs:
        lines.append("Aucun bug : chaque élément cliqué a produit une transition attendue.")
    else:
        lines += ["| # | Gravité | Écran | Élément | Ce qui s'est passé | Étapes | Preuve |",
                  "|---|---|---|---|---|---|---|"]
        for bug in bugs:
            lines.append(
                f"| {bug['id']} | {bug['severity']} | {bug['screen']} | {bug['element']} | "
                f"{bug['outcome']} — {bug['steps'][-1]} | {' · '.join(bug['steps'])} | "
                f"{bug['evidence'] or '—'} |"
            )
    lines += ["", "## Écrans", ""]
    by_screen: dict[str, list[dict]] = defaultdict(list)
    for row in run.get("rows", []):
        by_screen[row["screen"]["name"]].append(row)
    for screen in run.get("screens", []):
        rows = by_screen.get(screen["name"], [])
        lines.append(f"### {screen['name']} — `{screen['hash']}` — {screen['elements']} éléments")
        lines.append("")
        if not rows:
            lines.append("Aucun élément cliqué sur cet écran.")
            lines.append("")
            continue
        lines += ["| Élément (rôle + nom) | Action | Résultat | ms | Preuve / reproduire |",
                  "|---|---|---|---|---|"]
        for row in rows:
            result = "**NOTHING**" if row["outcome"] == "nothing" else row["outcome"]
            evidence = row.get("evidence") or (row.get("detail") or "")
            lines.append(f"| {row['element']['role']} « {row['element']['name']} » | "
                         f"{row['action']} | {result} | {row['ms']} | {evidence} |")
        lines.append("")
    lines += ["## Éléments non cliqués", "",
              f"- `duplicates={totals['duplicates']}` (règle du frère, §4.4)"]
    unreachable = [row for row in run.get("rows", []) if row["outcome"] == "unreachable"]
    lines.append(f"- `unreachable={len(unreachable)}`" + (
        " : " + ", ".join(f"`{row['screen']['name']}` « {row['element']['name']} »"
                          for row in unreachable[:10]) if unreachable else ""))
    skipped = [row for row in run.get("rows", []) if row["outcome"] == "skipped-session"]
    lines.append(f"- `skipped-session={len(skipped)}`" + (
        " : " + ", ".join(f"« {row['element']['name']} »" for row in skipped[:10]) if skipped else ""))
    hors = run.get("hors_passe", [])
    if hors:
        lines.append("- `hors-passe` : " + ", ".join(f"`{name}` ({why})" for name, why in hors))
    if run.get("label_collision"):
        lines.append(f"- `libellés partagés={run['label_collision']}` : deux écrans portent le même "
                     "titre, séparés par le hash de leurs textes (§9.5)")
    lines.append("")
    return "\n".join(lines)


def save_run(run: dict, out: Path) -> None:
    out.mkdir(parents=True, exist_ok=True)
    run["bugs"] = build_bugs(run.get("rows", []))
    run["summary"] = summary(run)
    (out / "bughunt-native.json").write_text(json.dumps(run, ensure_ascii=False, indent=1),
                                             encoding="utf-8")
    (out / "bughunt-native.md").write_text(render_md(run), encoding="utf-8")
    apk = run.get("apk", {})
    lines = [
        summary_line(run),
        f"tool={run.get('tool')}",
        f"head_run={run.get('head')}",
        f"apk={apk.get('name', '')} bytes={apk.get('bytes', '')} signer={apk.get('signer', '')} "
        f"sha256={apk.get('sha256', '')}",
    ]
    if summary(run)["stopped"]:
        lines.append(f"stopped={summary(run)['stopped']}")
    (out / "run.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")


def load_run(out: Path) -> dict:
    path = out / "bughunt-native.json"
    if path.is_file():
        try:
            return json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            pass
    return {}


def new_run(out: Path, head: str) -> dict:
    run = load_run(out)
    run.setdefault("app", APP)
    run.setdefault("run_id", str(int(time.time())))
    run.setdefault("tool", f"{TOOL}@{git_sha()}")
    run.setdefault("head", head)
    run.setdefault("date", now_iso())
    run.setdefault("rows", [])
    run.setdefault("screens", [])
    run.setdefault("elements", 0)
    run.setdefault("duplicates", 0)
    run.setdefault("scenarios", {})
    run.setdefault("hors_passe", [])
    run.setdefault("label_collision", 0)
    return run


# --------------------------------------------------------------------------
# the local checks that run without an emulator (§9.9)
# --------------------------------------------------------------------------

NEO_DUMP = """<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>
<hierarchy rotation="0">
  <node index="0" class="android.widget.FrameLayout" package="fr.neezer.mobile" text="" content-desc="" clickable="false" scrollable="true" bounds="[0,0][1080,2400]">
    <node index="0" class="android.view.View" package="fr.neezer.mobile" text="" content-desc="" clickable="true" scrollable="false" bounds="[0,300][1080,420]">
      <node index="0" class="android.widget.TextView" package="fr.neezer.mobile" text="Écouter" content-desc="" clickable="false" scrollable="false" bounds="[60,330][300,400]" />
    </node>
  </node>
  <node index="1" class="android.widget.TextView" package="fr.neezer.mobile" text="Accueil" content-desc="Accueil Neezer" clickable="false" scrollable="false" bounds="[60,80][400,140]" />
</hierarchy>
"""


def self_test() -> int:
    """The checks that run on a worker with no KVM: parse, dedupe, classify (§9.9)."""
    global PRECHECK
    PRECHECK = load_precheck()
    # parse_tree, on the embedded dump: the screen's name, the clickable row
    # under a scrollable, and its text line folded into that row.
    dump = Dump(NEO_DUMP, 120)
    if not dump.ok or len(dump.nodes) != 4:
        print(f"self_test: parse=FAIL nodes={len(dump.nodes)}")
        return 1
    if dump.name != "Accueil Neezer":
        print(f"self_test: parse=FAIL name={dump.name!r}")
        return 1
    kept, suppressed = elements(dump)
    if len(kept) != 1 or suppressed != 1:
        print(f"self_test: dedupe=FAIL kept={len(kept)} duplicates={suppressed}")
        return 1
    # The sibling rule: 12 siblings → 3 kept, 9 duplicate (§4.4). The scroll
    # container above them is not an element: its children are.
    rows = "\n".join(
        f'    <node index="{index}" class="android.view.View" package="{PKG}" text="" '
        f'content-desc="Titre {index}" clickable="true" scrollable="false" '
        f'bounds="[0,{400 + index * 120}][1080,{500 + index * 120}]" />'
        for index in range(12)
    )
    siblings = ("<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>\n"
                '<hierarchy rotation="0">\n'
                f'  <node index="0" class="android.widget.FrameLayout" package="{PKG}" text="" '
                'content-desc="Bibliothèque Neezer" clickable="false" scrollable="true" '
                'bounds="[0,0][1080,2400]">\n' + rows + "\n  </node>\n</hierarchy>\n")
    kept, suppressed = elements(Dump(siblings, 120))
    if len(kept) != 3 or suppressed != 9:
        print(f"self_test: dedupe=FAIL kept={len(kept)} duplicates={suppressed}")
        return 1
    # §4.1: one hand-built transition per class the classifier decides.
    base = {"name": "Réglages", "hash": "aaaa", "texts": ["Réglages", "Compte", "Sources"],
            "nodes": (("android.view.View", "Réglages", "", False, False, True),), "textual": 5,
            "layer": (), "ok": True, "ms": 120, "ime": False}
    cases = [
        ("exception", dict(base, name="Réglages", texts=[], nodes=(), textual=0, ok=False), ["FATAL"], "1"),
        ("blank", dict(base, texts=[], nodes=(), textual=0), [], "1"),
        ("freeze", dict(base, ok=False, ms=9000), [], "1"),
        ("navigated", dict(base, name="Compte", texts=["Compte"]), [], "1"),
        ("opened-layer", dict(base, layer=(("a",), ("b",), ("c",), ("d",))), [], "1"),
        ("state-changed", dict(base, nodes=(("android.view.View", "Réglages", "", True, False, True),)),
         [], "1"),
    ]
    for expected, after, crash, pid in cases:
        got, _, _ = classify(dict(base), dict(after), crash, pid)
        if got != expected:
            print(f"self_test: classify=FAIL expected={expected} got={got}")
            return 1
    got, _, _ = classify(dict(base), dict(base), [], "1")
    if got != "nothing":
        print(f"self_test: classify=FAIL expected=nothing got={got}")
        return 1
    # §4.1 native: a window the previous read did not name is a layer, even with
    # an unchanged tree (the full-screen player, measured on run 45915988351).
    got, _, _ = classify(dict(base, windows=()), dict(base, windows=("player",)), [], "1")
    if got != "opened-layer":
        print(f"self_test: classify=FAIL expected=opened-layer(window) got={got}")
        return 1
    got, _, _ = classify(dict(base, windows=("player",)), dict(base, windows=("player",)), [], "1")
    if got != "nothing":
        print(f"self_test: classify=FAIL expected=nothing(same window) got={got}")
        return 1
    # The window manager's own text, as `dumpsys window windows` prints it.
    raw = ("  Window #0 Window{1a2b3c u0 StatusBar}:\n"
           "  Window #1 Window{9f8e7d u0 fr.neezer.mobile/fr.neezer.mobile.MainActivity}:\n"
           "  Window #2 Window{aa11bb u0 InputMethod}:\n")
    if parse_windows(raw) != ("fr.neezer.mobile/fr.neezer.mobile.MainActivity",):
        print(f"self_test: windows=FAIL {parse_windows(raw)}")
        return 1
    # §9.6: the tree already read is the next element's « avant » on an
    # unchanged screen — and NEVER when the screen it belongs to is another one.
    walker = Walker(Adb("self-test"), Path(tempfile.gettempdir()), 10, 1)
    cached = Dump(NEO_DUMP, 120)
    first = Screen(1, "Accueil Neezer", "aaaa", [])
    walker.remember(first, cached)
    signature = elements(cached)[0][0].signature
    got = walker.resolve(first, signature)
    if got is None or got[0] is not cached:
        print("self_test: tree=FAIL the cached tree was not reused")
        return 1
    walker.remember(Screen(2, "Ailleurs", "bbbb", []), cached)
    if walker.resolve(first, signature) is not None:
        print("self_test: tree=FAIL another screen's tree was reused")
        return 1
    # §4.5: the destructive regimes, decided once.
    if destructive_of("Déconnexion") != ("passe", False):
        print("self_test: classify=FAIL destructive Déconnexion")
        return 1
    if destructive_of("Mettre à jour") != ("jamais", False):
        print("self_test: classify=FAIL destructive Mettre à jour")
        return 1
    # §4.4: the summary line the judge reads: percentage AND raw count.
    line = summary_line({"screens": [{"name": "x", "hash": "a", "elements": 1}], "elements": 1,
                         "rows": [{"outcome": "state-changed",
                                   "element": {"name": "Lire"}}]})
    if "coverage=100.0% (1/1)" not in line or "named_nothing=0" not in line:
        print(f"self_test: cover=FAIL {line}")
        return 1
    found, declared = check_routes(quiet=True)
    print(f"self_test: routes={found}/{declared} parse=ok dedupe=ok classify=ok")
    return 0 if found == declared and found > 0 else 1


def read_routes() -> tuple[list[str], list[tuple[str, str]]]:
    declared: list[str] = []
    hors: list[tuple[str, str]] = []
    for raw in Path(__file__).with_name("expected-routes.txt").read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split(None, 1)
        declared.append(parts[0])
        if len(parts) > 1 and "hors-passe" in parts[1]:
            hors.append((parts[0], parts[1].split(":", 1)[-1].strip()))
    return declared, hors


def check_routes(quiet: bool = False) -> tuple[int, int]:
    """`NavGraph.kt`'s `object Routes` against `expected-routes.txt` (§9.9)."""
    base = Path(__file__).resolve().parents[1]
    kotlin = base / "../android/app/src/main/java/fr/neezer/mobile/app/NavGraph.kt"
    declared, _ = read_routes()
    if not kotlin.is_file():
        print(f"bughunt: {kotlin} absent", file=sys.stderr)
        raise SystemExit(2)
    block = re.search(r"object\s+Routes\s*\{(.*?)\n\}", kotlin.read_text(encoding="utf-8"), re.S)
    found = re.findall(r"data\s+(?:object|class)\s+([A-Za-z0-9_]+)", block.group(1) if block else "")
    undeclared = [name for name in found if name not in declared]
    absent = [name for name in declared if name not in found]
    if undeclared or absent:
        print(f"routes {len(found) - len(undeclared)}/{len(declared)} declared")
        for name in undeclared:
            print(f"undeclared: {name} (dans NavGraph.kt, absent de expected-routes.txt)")
        for name in absent:
            print(f"missing: {name} (déclaré, absent de NavGraph.kt)")
        raise SystemExit(2)
    if not quiet:
        print(f"routes {len(declared)}/{len(declared)} declared")
    return len(found), len(declared)


# --------------------------------------------------------------------------
# main
# --------------------------------------------------------------------------

def main(argv: list[str]) -> int:
    global PRECHECK
    parser = argparse.ArgumentParser(description="La chasse aux bugs native (plan-bughunt §9).")
    parser.add_argument("--serial", help="le serial adb (exigé pour une passe)")
    parser.add_argument("--out", help="le dossier des preuves (défaut : un mktemp)")
    parser.add_argument("--only", help="ne rejoue qu'un écran découvert, par le nom du rapport")
    parser.add_argument("--scenario", choices=("burst", "settings", "session"),
                        help="ne joue qu'un scénario (§9.8)")
    parser.add_argument("--max-nodes", type=int, default=700)
    parser.add_argument("--max-minutes", type=int, default=37)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--check-routes", action="store_true")
    args = parser.parse_args(argv)

    PRECHECK = load_precheck()
    if args.self_test:
        return self_test()
    if args.check_routes:
        check_routes()
        return 0
    if not args.serial:
        parser.error("--serial est exigé (ou --self-test / --check-routes)")

    out = Path(args.out) if args.out else Path(tempfile.mkdtemp(prefix="neezer-bughunt-"))
    out.mkdir(parents=True, exist_ok=True)
    head = os.environ.get("NEEZER_HEAD_SHA") or git_sha()
    adb = Adb(args.serial)
    walker = Walker(adb, out, args.max_nodes, args.max_minutes)
    device = adb.device_description()
    walker.dpi = int(device.get("density") or 420)
    run = new_run(out, head)
    run["device"] = device
    run["apk"] = {
        "name": os.environ.get("NEEZER_APK_NAME", ""),
        "bytes": os.environ.get("NEEZER_APK_BYTES", ""),
        "signer": os.environ.get("NEEZER_APK_SIGNER", ""),
        "sha256": os.environ.get("NEEZER_APK_SHA256", ""),
    }
    (out / "device.txt").write_text(
        "\n".join(f"{key}={value}" for key, value in device.items()) + "\n", encoding="utf-8")
    print(f"bughunt: out={out} serial={args.serial} device={device['model']} api={device['sdk']} "
          f"density={device['density']}", flush=True)

    if args.scenario:
        started = time.monotonic()
        if args.scenario == "burst":
            verdict = scenario_burst(walker)
            run["scenarios"]["burst"] = verdict
            run["scenarios"]["burst-alternated"] = dict(verdict,
                                                        name=SCENARIO_LABEL["burst-alternated"])
        elif args.scenario == "settings":
            verdict = scenario_settings(walker)
            run["scenarios"]["settings"] = verdict
        else:
            verdict = scenario_session(walker)
            run["scenarios"]["session"] = verdict
        run["seconds"] = run.get("seconds", 0) + int(time.monotonic() - started)
        run["hors_passe"] = read_routes()[1]
        save_run(run, out)
        print(f"scenario={args.scenario} verdict={verdict['verdict']} {verdict['numbers']}",
              flush=True)
        if verdict.get("line"):
            print(verdict["line"], flush=True)
        return 0

    if args.only:
        # Rejouer UN écran : ses chemins sont ceux de la passe précédente, lue
        # dans le JSON (c'est le seul mode qui ne repart pas de zéro).
        previous = load_run(out)
        for item in previous.get("screens", []):
            screen = Screen(item.get("index", len(walker.screens) + 1), item["name"],
                            item.get("hash", ""), [tuple(point) for point in item.get("path", [])],
                            item.get("elements", 0), item.get("duplicates", 0))
            walker.screens.append(screen)
            walker.by_name[screen.name] = screen
        walker.rows = [row for row in previous.get("rows", [])
                       if row["screen"]["name"] != args.only]

    started = time.monotonic()
    run_pass(walker, args.only)
    run["seconds"] = int(time.monotonic() - started)
    run["rows"] = walker.rows
    run["screens"] = [
        {"index": screen.index, "name": screen.name, "hash": screen.hash,
         "elements": screen.elements, "duplicates": screen.dup,
         "path": [list(point) for point in screen.path], "restore": screen.restore}
        for screen in walker.screens
    ]
    run["elements"] = sum(screen.elements for screen in walker.screens)
    run["duplicates"] = walker.suppressed
    run["stopped"] = walker.stopped
    run["capped"] = walker.capped
    run["label_collision"] = walker.label_collision
    run["hors_passe"] = read_routes()[1]
    save_run(run, out)

    # The evidence a person reads after the report (the T9 names).
    (out / "logcat.txt").write_text(adb.shell("logcat", "-d", timeout=60), encoding="utf-8")
    crash = adb.crash_lines()
    (out / "crash.txt").write_text("\n".join(crash) + ("\n" if crash else ""), encoding="utf-8")

    print(summary_line(run), flush=True)
    print(f"bughunt: report={out / 'bughunt-native.md'}", flush=True)
    return 1 if any(row["outcome"] == "exception" for row in walker.rows) else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
