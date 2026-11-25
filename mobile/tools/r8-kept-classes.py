#!/usr/bin/env python3
"""Every class a `-keep` rule promises to keep is in the shipped APK, unrenamed.

    mobile/tools/r8-kept-classes.py \
        --mapping mobile/android/app/build/outputs/mapping/release/mapping.txt \
        --apk mobile/android/app/build/outputs/apk/release/app-universal-release.apk \
        mobile/android/app/proguard-rules.pro mobile/android/bridge/consumer-rules.pro

Why a general check and not a second per-class grep
---------------------------------------------------
`mobile.yaml` already greps the mapping for the three `rustls-platform-verifier`
classes (the v1.6.5 bug). That check is a fast pre-check for ONE bug: it names
its classes by hand, so the next reflective edge R8 strips is not covered until
somebody writes another literal. This script reads the SOURCE OF TRUTH — every
module's `-keep` rules — and proves each promise on the artifact a person
installs:

  1. each `-keep class <pattern> { ... }` rule matches at least one class in
     R8's mapping (a rule that names nothing is dead config or a renamed
     target), and
  2. every class it matches is kept under its original name (`X -> X:` in the
     mapping, never `X -> a.b:`), and
  3. its dex descriptor (`LX;`) is in the shipped APK's string table.

Conditional keeps (`-keep class * implements ...`, `... extends ...`) are
skipped: which classes satisfy them is a fact of the graph, not of a name, and
the mapping does not record interfaces. `-keepclassmembers` is skipped too: it
keeps members, not the class's name.

The inputs are the two files `mobile.yaml`'s `apk` job already produces
(`assembleRelease` writes both). No network, no build, stdlib only.

Sources
-------
- Android, *Shrink, obfuscate, and optimize your app* — the mapping file and
  what a keep rule guarantees:
  <https://developer.android.com/build/shrink-code#keep-code>
- Android, *Test and troubleshoot the optimization* — a reflective call crashes
  as `ClassNotFoundException` / `NoSuchMethodError`, and "the mapping.txt is the
  truth": <https://developer.android.com/topic/performance/app-optimization/test-and-troubleshoot-the-optimization>
- ProGuard, *Keep options* — `-keep` vs `-keepclassmembers`, and the `*` / `**`
  wildcards: <https://www.guardsquare.com/manual/configuration/usage#keepoptions>
"""

from __future__ import annotations

import argparse
import re
import sys
import zipfile

# A class line in R8's mapping.txt starts at column 0 ("com.foo.Bar -> a.b:"),
# and its members are indented under it. Anchored, so a member line never
# passes for a class.
CLASS_LINE = re.compile(r"^(\S+) -> (\S+):$")

# R8 and the Kotlin compiler SYNTHESISE classes after the keep rules are read:
# `Foo$$ExternalSyntheticLambda3` (a lambda R8 replaced with a direct call) and
# `Foo$$ExternalSyntheticOutline0` (shared string building). They are never named
# in source and never reached reflectively, so R8 is free to rename them — the
# build measured 102 of them under `-keep class uniffi.**` / `fr.neezer.mobile
# .bridge.**`, and every one was renamed while every hand-written class kept its
# name. The check skips them and says how many it skipped; it does not lower the
# bar for the classes a person or JNI actually names.
SYNTHETIC = re.compile(r"\$\$(?:ExternalSynthetic|Lambda\$)")

# `-keep class <spec>`, `-keep,allowoptimization class <spec>`, and this repo's
# spelling with a space after the comma (`-keep, includedescriptorclasses class
# ...`). The spec ends at the opening brace of the member block, or at end of line.
KEEP_RULE = re.compile(r"^\s*-keep(?:\s*,\s*[a-z]+)*\s+class\s+([^\{]+?)\s*(?:\{.*)?$")


def keep_specs(paths: list[str]) -> list[tuple[str, str]]:
    """(spec, rule-file) for every unconditional `-keep class` a module declares."""
    out: list[tuple[str, str]] = []
    for path in paths:
        with open(path, encoding="utf-8") as f:
            for line in f:
                line = line.split("#", 1)[0].rstrip()
                if not line:
                    continue
                m = KEEP_RULE.match(line)
                if not m:
                    continue
                spec = m.group(1).strip()
                # Conditional keeps cannot be resolved from a mapping (it names
                # no interfaces), and a member-only rule keeps no class name.
                if " implements " in spec or " extends " in spec:
                    continue
                out.append((spec, path))
    return out


def spec_to_regex(spec: str) -> re.Pattern[str]:
    """ProGuard wildcards: `**` crosses package dots, `*` does not."""
    parts = []
    i = 0
    while i < len(spec):
        if spec.startswith("**", i):
            parts.append(".*")
            i += 2
        elif spec[i] == "*":
            parts.append("[^.]*")
            i += 1
        elif spec[i] == "?":
            parts.append("[^.]")
            i += 1
        else:
            parts.append(re.escape(spec[i]))
            i += 1
    return re.compile("^" + "".join(parts) + "$")


def read_mapping(path: str) -> dict[str, str]:
    """Original class name -> the name R8 gave it (equal when kept)."""
    out: dict[str, str] = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            m = CLASS_LINE.match(line)
            if m:
                out[m.group(1)] = m.group(2)
    return out


def dex_strings(apk: str) -> str:
    """The concatenated string tables of the APK's classes*.dex, one blob."""
    blob: list[str] = []
    with zipfile.ZipFile(apk) as z:
        for name in z.namelist():
            if re.fullmatch(r"classes\d*\.dex", name):
                data = z.read(name)
                # Pull printable ASCII runs; a descriptor only needs to be
                # present somewhere in the table.
                blob.append(
                    "".join(chr(b) if 32 <= b < 127 else "\n" for b in data)
                )
    return "\n".join(blob)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--mapping", required=True, help="R8's mapping.txt")
    ap.add_argument("--apk", required=True, help="the shipped APK")
    ap.add_argument("rules", nargs="+", help="every -keep rule file across modules")
    args = ap.parse_args()

    mapping = read_mapping(args.mapping)
    if not mapping:
        print(f"::error::{args.mapping} holds no class mapping", file=sys.stderr)
        return 1
    dex = dex_strings(args.apk)
    if not dex:
        print(f"::error::{args.apk} carries no classes*.dex", file=sys.stderr)
        return 1

    failures = 0
    checked = 0
    synthetic = 0
    for spec, rule_file in keep_specs(args.rules):
        pattern = spec_to_regex(spec)
        matched = [name for name in mapping if pattern.match(name)]
        if not matched:
            print(
                f"::error::the keep rule for <<{spec}>> ({rule_file}) matches no "
                f"class in the mapping: the rule is dead or the dependency is gone",
                file=sys.stderr,
            )
            failures += 1
            continue
        rule_broken = 0
        rule_synthetic = 0
        for name in sorted(matched):
            if SYNTHETIC.search(name):
                rule_synthetic += 1
                synthetic += 1
                continue
            checked += 1
            kept = mapping[name]
            if kept != name:
                print(
                    f"::error::R8 renamed the kept class {name} -> {kept} "
                    f"(<<{spec}>> in {rule_file}): a reflective edge will not find it",
                    file=sys.stderr,
                )
                rule_broken += 1
                continue
            descriptor = "L" + name.replace(".", "/") + ";"
            if descriptor not in dex:
                print(
                    f"::error::the shipped APK has no {descriptor} for the kept "
                    f"class {name} (<<{spec}>> in {rule_file})",
                    file=sys.stderr,
                )
                rule_broken += 1
        failures += rule_broken
        if rule_broken == 0:
            extra = f", {rule_synthetic} synthetic skipped" if rule_synthetic else ""
            print(
                f"   kept: {spec} -> {len(matched) - rule_synthetic} class(es) "
                f"present and unrenamed{extra}"
            )

    if failures:
        print(
            f"::error::R8 keep rules: {failures} promise(s) broken across {len(args.rules)} file(s)",
            file=sys.stderr,
        )
        return 1
    print(
        f"r8_kept_classes=1 specs={len(keep_specs(args.rules))} classes={checked} "
        f"synthetic_skipped={synthetic} ship={args.apk.rsplit('/', 1)[-1]}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
