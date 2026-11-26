#!/usr/bin/env python3
"""What a script can count in a `mobile-journey-evidence` directory.

    python mobile/tools/judge/precheck.py <evidence-dir>

The judge (`mobile/tools/judge/rubric.md`) reads pixels; this file reads the
accessibility tree, the crash buffer and the device description, and prints ONE
line per check so a review can cite it by name:

    screens 11/11 ok
    crash empty ok
    margin 01-accueil left=20 right=20 ok
    overflow 01-accueil nodes=0 ok
    overlap 01-accueil clickable_pairs=0 ok
    clipped 01-accueil n=0 ok
    targets 01-accueil below_48dp=0 ok
    banned 01-accueil hits=0 ok
    …
    precheck: ok=68 fail=0

Why it exists (plan-mobile-tests, lot T9): a criterion that lives only in a
prompt drifts. Six of the rubric's blocking rules are countable from the tree
and the buffers — this is where they are written down, and the rubric cites
these exact line names. The judge keeps the part a person must look at: does
this screen look like the web app.

Rules, each one a measured reality of `uiautomator dump`:

  - the viewport is the root node's own bounds; `dp = px × 160 / dpi`, dpi from
    `device.txt` (`density=420` on the AVD, pixel_6);
  - a scroll container reports its children UNCLIPPED (this is why `ui.sh` taps
    a node's upper part): a node inside a `scrollable="true"` ancestor is not an
    overflow, and a genuine off-screen control still is;
  - the status and navigation bars (`package="com.android.systemui"`) are not
    the app's layout: they are exempt from margins and overflow;
  - a node that spans the whole width is a background or a row, not a "block":
    the margin is measured on the nodes that carry a word a person reads;
  - nesting is not an overlap: a clickable inside another clickable is the
    row-and-its-thumbnail shape every list here has.

Exit status is 0 whatever the checks say: the judge reads the lines, and a
non-zero exit would look like a crash of the tool rather than a finding. Only a
missing or unreadable directory is a usage error (exit 2, no verdict line).

Python 3.12 standard library only (`re`, `pathlib`): the judge's task must be
runnable on any worker with no install step. The dump is read by a small,
explicit scanner (`parse_tree`) rather than an XML library: the one attribute
per tag, one `node` per element shape of a `uiautomator dump` is all this file
needs, it keeps the parser free of DTD/entity expansion by construction, and it
adds no dependency to a task the judge runs anywhere.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

# The eleven captures of `release-journey.sh --evidence`, in walk order. The
# rubric's screen table uses the same names, so a citation is unambiguous.
SCREENS = (
    "01-accueil",
    "02-rechercher",
    "03-bibliotheque",
    "04-podcasts",
    "05-reglages",
    "06-lecture-et-son",
    "07-mode-voiture",
    "08-lecteur",
    "09-lecteur-suivant",
    "10-file",
    "11-lecteur-glisse",
)

# The developer-text list of the rubric (C5). `\bMB[0-9]+\b` is this repo's own
# lot naming ("MB3a"), `snake_case` a Kotlin field that leaked, and
# `[a-z][A-Z][a-z]` a camelCase identifier shown to a person.
BANNED = re.compile(
    r"TODO|FIXME|lorem|stub|null|Exception|Traceback|\bMB[0-9]+\b|snake_case|[a-z][A-Z][a-z]"
)

BOUNDS = re.compile(r"\[(-?\d+),(-?\d+)\]\[(-?\d+),(-?\d+)\]")
SYSTEM_UI = "com.android.systemui"
# A PNG smaller than this is a truncated `exec-out screencap` (the escalation in
# the lot): a real 1080×2400 screenshot is 100 kB to 2 MB.
MIN_PNG_BYTES = 10_000
# The AVD's own density (`ro.sf.lcd_density=420`, pixel_6): used when
# device.txt does not carry one, so a dp is never silently a pixel.
DEFAULT_DPI = 420


class Node:
    __slots__ = ("text", "desc", "cls", "package", "clickable", "scrollable",
                 "visible", "x1", "y1", "x2", "y2", "parent", "has_ancestor")

    def __init__(self, attrs: dict[str, str], parent: "Node | None") -> None:
        self.text = attrs.get("text", "")
        self.desc = attrs.get("content-desc", "")
        self.cls = attrs.get("class", "")
        self.package = attrs.get("package", "")
        self.clickable = attrs.get("clickable") == "true"
        self.scrollable = attrs.get("scrollable") == "true"
        # `visible-to-user` exists on API 30+ dumps; when it is absent the node
        # is visible (uiautomator only dumps what is on screen).
        self.visible = attrs.get("visible-to-user", "true") != "false"
        m = BOUNDS.search(attrs.get("bounds", ""))
        self.x1, self.y1, self.x2, self.y2 = (int(v) for v in m.groups()) if m else (0, 0, 0, 0)
        self.parent = parent

    @property
    def w(self) -> int:
        return max(0, self.x2 - self.x1)

    @property
    def h(self) -> int:
        return max(0, self.y2 - self.y1)

    def label(self) -> str:
        word = (self.text or self.desc or "").strip()
        return f'{self.cls}:"{word[:40]}"' if word else self.cls


class DumpError(ValueError):
    """The file is not a `uiautomator dump`."""


# One element per line in a uiautomator dump: `<node …>` opens, `<node … />` is a
# leaf, `</node>` closes. The attribute part never contains a raw `>` — the tool
# escapes it — so `[^>]*` is exact.
TAG = re.compile(r"<node\b([^>]*?)(/?)>|</node\s*>")
ATTR = re.compile(r'([A-Za-z-]+)="([^"]*)"')
# uiautomator escapes exactly these five (`&#39;` for a quote) — the same set
# `mobile/tools/ui.sh` undoes before matching a label.
ENTITIES = (("&lt;", "<"), ("&gt;", ">"), ("&quot;", '"'), ("&#39;", "'"), ("&amp;", "&"))


def decode(value: str) -> str:
    for entity, char in ENTITIES:
        value = value.replace(entity, char)
    return value


def parse_tree(path: Path) -> tuple[Node, list[Node]]:
    """The dumped hierarchy → (root, every node in document order).

    NO XML LIBRARY, on purpose. A dump is one `<node>` element per line with a
    fixed attribute shape, and reading it this way is what keeps the judge's
    task dependency-free (`defusedxml` would otherwise be needed: a parser that
    honours a DTD is a parser an artifact can bomb). A `DOCTYPE`/`ENTITY`
    declaration is refused outright instead of being expanded.
    """
    raw = path.read_text(encoding="utf-8", errors="replace")
    if "<!DOCTYPE" in raw or "<!ENTITY" in raw:
        raise DumpError("a DOCTYPE/ENTITY declaration is not a uiautomator dump")

    nodes: list[Node] = []
    stack: list[Node] = []
    for match in TAG.finditer(raw):
        if match.group(0).startswith("</"):
            if not stack:
                raise DumpError("a closing </node> with nothing open")
            stack.pop()
            continue
        attrs = {name: decode(value) for name, value in ATTR.findall(match.group(1))}
        node = Node(attrs, stack[-1] if stack else None)
        nodes.append(node)
        if not match.group(2):
            stack.append(node)
    if stack:
        raise DumpError("a <node> left open at the end of the dump")
    if not nodes:
        raise DumpError("no <node> in the dump")
    return nodes[0], nodes


def inside_scrollable(node: Node) -> bool:
    """A scroll container reports its children unclipped: not an overflow."""
    parent = node.parent
    while parent is not None:
        if parent.scrollable:
            return True
        parent = parent.parent
    return False


def density(directory: Path) -> int:
    device = directory / "device.txt"
    if device.is_file():
        text = device.read_text(encoding="utf-8", errors="replace")
        for pattern in (r"\bdensity=(\d+)\b", r"ro\.sf\.lcd_density=(\d+)\b", r"\b(\d{3})dpi\b"):
            m = re.search(pattern, text)
            if m and int(m.group(1)) > 0:
                return int(m.group(1))
    return DEFAULT_DPI


def dp(px: int, dpi: int) -> float:
    return px * 160.0 / dpi


def screen_check(name: str, directory: Path, dpi: int, ok: list[str], fail: list[str]) -> None:
    path = directory / "screens" / f"{name}.xml"
    root, nodes = parse_tree(path)
    width, height = root.x2, root.y2
    if width <= 0 or height <= 0:
        fail.append(f'margin {name} FAIL bounds="{root.x1},{root.y1},{root.x2},{root.y2}"')
        return

    app = [n for n in nodes if n.visible and n.package != SYSTEM_UI]

    # --- C2: the page's margin, left against right --------------------------
    # The blocks are the nodes that carry a word; a node spanning the viewport
    # is a background or a full-bleed row, not a margin.
    blocks = [n for n in app if (n.text or n.desc) and not (n.x1 <= 2 and n.x2 >= width - 2)]
    if len(blocks) < 2:
        fail.append(f"margin {name} left=0 right=0 nodes={len(blocks)} FAIL (no text block to measure)")
    else:
        left = round(dp(min(n.x1 for n in blocks), dpi))
        right = round(dp(width - max(n.x2 for n in blocks), dpi))
        bad = abs(left - right) > 2 or min(left, right) < 12
        (fail if bad else ok).append(
            f"margin {name} left={left} right={right} {'FAIL' if bad else 'ok'}"
        )

    # --- C2: nothing outside the viewport -----------------------------------
    overflow = [
        n for n in app
        if not inside_scrollable(n) and (n.x2 > width or n.y2 > height)
    ]
    (fail if overflow else ok).append(
        f"overflow {name} nodes={len(overflow)} "
        + (f'FAIL first={overflow[0].label()}' if overflow else "ok")
    )

    # --- C2: a control hidden under another ---------------------------------
    clicks = [n for n in app if n.clickable and n.w > 0 and n.h > 0]
    pairs = []
    for i, a in enumerate(clicks):
        for b in clicks[i + 1:]:
            ix = min(a.x2, b.x2) - max(a.x1, b.x1)
            iy = min(a.y2, b.y2) - max(a.y1, b.y1)
            if ix <= 0 or iy <= 0 or ix * iy <= 64:
                continue
            a_in_b = a.x1 >= b.x1 and a.y1 >= b.y1 and a.x2 <= b.x2 and a.y2 <= b.y2
            b_in_a = b.x1 >= a.x1 and b.y1 >= a.y1 and b.x2 <= a.x2 and b.y2 <= a.y2
            if a_in_b or b_in_a:
                continue  # a row and its own button: nesting, not a collision
            pairs.append((a, b))
    (fail if pairs else ok).append(
        f"overlap {name} clickable_pairs={len(pairs)} "
        + (f"FAIL first={pairs[0][0].label()} / {pairs[0][1].label()}" if pairs else "ok")
    )

    # --- C2/C5: a label the layout had to truncate --------------------------
    clipped = [
        n for n in app
        if n.text.strip().endswith("…") and len(n.desc.strip()) > len(n.text.strip())
    ]
    (fail if clipped else ok).append(
        f"clipped {name} n={len(clipped)} "
        + (f"FAIL first={clipped[0].label()}" if clipped else "ok")
    )

    # --- C2: the design system's 48 dp floor --------------------------------
    edge = 48 * dpi / 160.0
    small = [n for n in app if n.clickable and (n.w < edge or n.h < edge)]
    (fail if small else ok).append(
        f"targets {name} below_48dp={len(small)} "
        + (f"FAIL first={small[0].label()} {round(dp(small[0].w, dpi))}x{round(dp(small[0].h, dpi))}dp"
           if small else "ok")
    )

    # --- C5: developer text on a person's screen ----------------------------
    hits = []
    for n in app:
        for value in (n.text, n.desc):
            m = BANNED.search(value)
            if m:
                hits.append((value, m.group(0)))
                break
    (fail if hits else ok).append(
        f"banned {name} hits={len(hits)} "
        + (f'FAIL first="{hits[0][0][:60]}" matched={hits[0][1]}' if hits else "ok")
    )


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: precheck.py <evidence-dir>", file=sys.stderr)
        return 2
    directory = Path(argv[1])
    if not directory.is_dir():
        print(f"precheck.py: {directory} is not a directory", file=sys.stderr)
        return 2

    dpi = density(directory)
    ok: list[str] = []
    fail: list[str] = []

    # --- the eleven captures ------------------------------------------------
    missing = []
    usable = []
    for name in SCREENS:
        png = directory / "screens" / f"{name}.png"
        xml = directory / "screens" / f"{name}.xml"
        png_ok = png.is_file() and png.stat().st_size > MIN_PNG_BYTES
        xml_ok = False
        if xml.is_file():
            try:
                parse_tree(xml)
                xml_ok = True
            except (DumpError, OSError):
                xml_ok = False
        if png_ok and xml_ok:
            usable.append(name)
        else:
            missing.append(
                f"{name}("
                + ", ".join(
                    part
                    for part, good in (
                        ("png" if not png.is_file() else f"png {png.stat().st_size} B", png_ok),
                        ("xml" if not xml.is_file() else "xml unparsable", xml_ok),
                    )
                    if not good
                )
                + ")"
            )
    if missing:
        fail.append(f"screens {len(usable)}/{len(SCREENS)} FAIL missing={','.join(missing)}")
    else:
        ok.append(f"screens {len(usable)}/{len(SCREENS)} ok")

    # --- C1: the crash buffer ----------------------------------------------
    crash_file = directory / "crash.txt"
    crash_lines = []
    if crash_file.is_file():
        crash_lines = [line for line in crash_file.read_text(encoding="utf-8", errors="replace").splitlines() if line.strip()]
    if crash_lines:
        fail.append(f'crash FAIL lines={len(crash_lines)} first="{crash_lines[0][:120]}"')
    else:
        ok.append("crash empty ok")

    # --- six countable criteria per screen that is fully there --------------
    for name in usable:
        screen_check(name, directory, dpi, ok, fail)

    for line in ok + fail:
        print(line)
    print(f"precheck: ok={len(ok)} fail={len(fail)}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
