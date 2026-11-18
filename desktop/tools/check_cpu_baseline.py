#!/usr/bin/env python3
"""Fail when code Neezer compiles needs more than the CPU floor it promises.

`docs/agents/desktop.md` gives the desktop **two** engine binaries, each with its own
floor, and this script checks either:

- **x86-64-v2** (SSE4.2 + POPCNT: every x86-64 CPU since Nehalem 2008 / Bulldozer 2011)
  — `neezer-transcribe`, the child every machine can run;
- **x86-64-v3** (AVX, AVX2, FMA, F16C, BMI2: Haswell 2013 / Excavator 2015) —
  `neezer-transcribe-v3`, which the supervisor starts when the CPU answers those three
  features. Measured: 5.0× real time at v2, 23.5× at v3, on the same excerpt.

whisper.cpp's CPU backend is the one part of the desktop stack whose instruction set is
decided at BUILD time: ggml defaults to `GGML_NATIVE=ON`, i.e. `-march=native`, so one
commit produces an AVX-512 binary on one builder and an SSE4.2 one on another — and
Swatinem/rust-cache happily restores the first into a CI job running on the second.
Measured in this repository: a ggml compiled on a Zen 4 laptop needs **104 194** AVX-512
instructions (EVEX) and dies with `SIGILL (signal 4)` under `qemu-x86_64 -cpu Nehalem`;
the same build with the flags pinned in `desktop/.cargo/config.toml` needs 0 and
transcribes. This script is the CHECK that the pin really reached the compiler.

**What is gated.** Only the code this repository compiles itself — symbols matching
`--gate` (`ggml*`, `whisper*` by default). Two dependencies legitimately contain
AVX-512 and would fail a naive census: aws-lc-rs ships `*_avx512` AES-GCM/CLMUL
kernels and rustfft ships `avx`/`avx2` planners, both reached only after a CPUID
check (they are *runtime-dispatched*, the thing x86-64-v2 hardware runs the baseline
branch of). They are counted and printed as `external`, never gated. The gate on the
whole shipped binary is behavioural: the engine itself, run under an emulated
x86-64-v2 CPU (see `--baseline` below and the `qemu` steps of the workflows).

**Why the raw instruction BYTES are read.** `objdump` prints `vpbroadcastq %rcx,%xmm0`
for both the VEX (AVX2) and the EVEX (AVX-512) encoding, so a mnemonic census cannot
see the instruction that killed whisper.cpp's own docker images (whisper.cpp#4103:
EVEX `vpbroadcastq`, no `%zmm` anywhere, SIGILL in
`ggml_backend_cpu_get_extra_buffer_types`). The prefix is in the bytes: EVEX starts
with `62`, VEX with `c4`/`c5`.

`lzcnt`/`tzcnt` are NOT gated: they are `bsr`/`bsf` with an F3 prefix, documented to
execute as those on CPUs without the feature (LLVM emits them for baseline x86-64 on
purpose). They cannot raise `SIGILL`.

Usage:
    # The objects the flags actually reached (symbols present, the real gate):
    python3 desktop/tools/check_cpu_baseline.py \\
        desktop/target/release/build/whisper-rs-sys-*/out/build/ggml/src
    # ...and the x86-64-v3 child's own objects, at ITS level:
    python3 desktop/tools/check_cpu_baseline.py --baseline x86-64-v3 \\
        desktop/target/engine-v3/release/build/whisper-rs-sys-*/out/build/ggml/src
    # A binary a package ships (stripped: count everything, gate nothing):
    python3 desktop/tools/check_cpu_baseline.py --report desktop/target/release/neezer-transcribe

Exit status: 0 inside the floor, 1 when gated code exceeds it, 2 when the input
cannot be read or attributed (a check that cannot run must never look like a pass).
"""

from __future__ import annotations

import argparse
import bisect
import os
import re
import subprocess
import sys

# --- categories ---------------------------------------------------------------
# Everything below is outside x86-64-v2 (SSE3, SSSE3, SSE4.1, SSE4.2 + POPCNT,
# CX16, LAHF/SAHF). The ones marked `trap` raise #UD (SIGILL) on a CPU without them.
DESCRIPTIONS = {
    "avx512": "AVX-512 (EVEX prefix, %zmm, mask registers) — Intel 2017+ / AMD 2022+",
    "avx": "AVX (VEX prefix or %ymm) — Sandy Bridge 2011 / Bulldozer 2011",
    "fma": "FMA3 (vfmadd*) — Haswell 2013 / Piledriver 2012",
    "f16c": "F16C (vcvtph2ps / vcvtps2ph) — Ivy Bridge 2012",
    "bmi2": "BMI2 (mulx, pdep, pext, rorx, sarx, shlx, shrx) — Haswell 2013",
    "movbe": "MOVBE — Atom 2013 / Haswell",
    "aes": "AES-NI — Westmere 2010",
    "pclmul": "PCLMULQDQ — Westmere 2010",
    "mpx": "MPX (bnd prefix) — not in x86-64-v2",
    # Informational: a subset of `avx`, and two encodings that are safe on old CPUs.
    "avx2": "AVX2-only integer instructions (informational: already counted as AVX)",
    "redundant": "lzcnt/tzcnt — bsr/bsf with an F3 prefix: safe on every x86-64",
}
INFORMATIONAL = {"avx2", "redundant"}
# The micro-architecture level each category belongs to. `2` is inside x86-64-v2
# (nothing to declare), `3` is what x86-64-v3 adds on top (AVX, AVX2, FMA, F16C,
# BMI2, MOVBE, LZCNT), `4` is everything above it (AVX-512, AES-NI, PCLMUL, MPX).
LEVEL = {
    "avx": 3,
    "avx2": 3,
    "fma": 3,
    "f16c": 3,
    "bmi2": 3,
    "movbe": 3,
    "avx512": 4,
    "aes": 4,
    "pclmul": 4,
    "mpx": 4,
}
LEVELS = {"x86-64-v2": 2, "x86-64-v3": 3}


def gated_categories(level: int) -> list[str]:
    """The categories that exceed a floor of `level` (2 or 3)."""
    return [
        key
        for key in DESCRIPTIONS
        if key not in INFORMATIONAL and LEVEL.get(key, 4) > level
    ]
SAMPLE_ORDER = ["avx512", "avx", "fma", "f16c", "bmi2", "movbe", "aes", "pclmul", "mpx"]

FMA = re.compile(r"^vfn?m(add|sub|addsub|subadd)")
F16C = {"vcvtph2ps", "vcvtps2ph"}
AVX2_ONLY = {
    "vpbroadcastb", "vpbroadcastw", "vpbroadcastd", "vpbroadcastq",
    "vperm2i128", "vpermd", "vpermps", "vpermq", "vpermpd",
    "vpsllvd", "vpsllvq", "vpsrlvd", "vpsrlvq", "vpsravd",
    "vpmaskmovd", "vpmaskmovq", "vpgatherdd", "vpgatherdq", "vpgatherqd",
    "vpgatherqq", "vgatherdps", "vgatherdpd", "vgatherqps", "vgatherqpd",
    "vpblendd", "vpshufb", "vpmulld", "vpackusdw", "vpcmpeqq", "vpcmpgtq",
    "vpaddq", "vpsubq",
}
BMI2 = {"bzhi", "mulx", "pdep", "pext", "rorx", "sarx", "shlx", "shrx"}
AVX512_ONLY = {
    "vpternlogd", "vpternlogq", "vpermi2b", "vpermi2w", "vpermi2d", "vpermi2q",
    "vpermt2b", "vpermt2w", "vpermt2d", "vpermt2q", "vpcompressb", "vpcompressw",
    "vpcompressd", "vpcompressq", "vpexpandb", "vpexpandw", "vpexpandd",
    "vpexpandq", "vpconflictd", "vpconflictq", "vplzcntd", "vplzcntq",
    "vpshldd", "vpshldq", "vpshldw", "vpshrdd", "vpshrdq", "vpshrdw",
    "vpmadd52luq", "vpmadd52huq", "vscalefps", "vscalefpd", "vrcp14ps",
    "vrcp14pd", "vrsqrt14ps", "vrsqrt14pd", "vgetexpps", "vgetexppd",
    "vgetmantps", "vgetmantpd", "vfixupimmps", "vfixupimmpd", "vreduceps",
    "vreducepd", "vrndscaleps", "vrndscalepd", "vfpclassps", "vfpclasspd",
    "vdbpsadbw", "vpermb", "vpermw", "vpmultishiftqb", "vpdpbusd",
    "vpdpbusds", "vpdpwssd", "vpdpwssds", "vpdpbssd", "vp2intersectd",
    "vp2intersectq", "kmovb", "kmovw", "kmovd", "kmovq", "kandb", "kandw",
    "kandd", "kandq", "korb", "korw", "kord", "korq", "kxorb", "kxorw",
    "kxord", "kxorq", "knotb", "knotw", "knotd", "knotq", "kortestb",
    "kortestw", "kortestd", "kortestq", "ktestb", "ktestw", "ktestd",
    "ktestq", "kshiftlb", "kshiftlw", "kshiftld", "kshiftlq", "kshiftrb",
    "kshiftrw", "kshiftrd", "kshiftrq", "kunpckbw", "kunpckwd", "kunpckdq",
    "kaddb", "kaddw", "kaddd", "kaddq",
}
# VEX-prefixed mnemonics that are not vector instructions (VMX/SVM/system).
NOT_AVX = {
    "verr", "verw", "vmread", "vmwrite", "vmptrld", "vmptrst", "vmclear",
    "vmcall", "vmlaunch", "vmresume", "vmxoff", "vmxon", "vmfunc", "vmrun",
    "vmload", "vmsave", "invlpga", "skinit", "stgi", "clgi",
}

def instruction(line: str) -> tuple[str, str, str] | None:
    """`(bytes, mnemonic, operands)` of one `objdump -d` line, or None.

    objdump separates its three columns with a TAB, and it WRAPS the bytes of a
    long instruction (a 10-byte `movabs`, an EVEX `vmovdqu32`) onto a
    continuation line that holds bytes and nothing else. Read as instructions,
    those fragments invent one: the measurement that made this function exist was
    `48 be 4b 59 86 38 d6` + a continuation `c5 6d 34` (the tail of the `movabs`
    immediate), which looked like a VEX instruction in a build with AVX off.
    """
    columns = line.split("\t")
    if len(columns) < 3 or not columns[2].strip():
        return None  # a wrapped byte fragment, or a section header
    text = columns[2].strip()
    mnemonic, _, operands = text.partition(" ")
    return columns[1].strip() + " ", mnemonic, operands


def classify(raw: str, mnemonic: str, operands: str) -> list[str]:
    """Every ISA category this one instruction needs (empty = inside the floor)."""
    found: list[str] = []
    opcode = raw.split(" ", 1)[0]
    name = mnemonic.rstrip(",")
    if name not in NOT_AVX:
        if opcode == "62":
            found.append("avx512")  # EVEX: AVX-512 at ANY operand width
        elif opcode in ("c4", "c5"):
            found.append("avx")  # VEX: AVX at any operand width
    if "zmm" in operands or "%k" in operands:
        found.append("avx512")
    if "ymm" in operands:
        found.append("avx")
    if name in AVX512_ONLY:
        found.append("avx512")
    if name in AVX2_ONLY:
        found.append("avx2")
    if FMA.match(name):
        found.append("fma")
    if name in F16C:
        found.append("f16c")
    if name in BMI2:
        found.append("bmi2")
    if name in ("lzcnt", "tzcnt"):
        found.append("redundant")
    if name == "movbe":
        found.append("movbe")
    if name in ("aesenc", "aesenclast", "aesdec", "aesdeclast", "aesimc",
                "aeskeygenassist"):
        found.append("aes")
    if name == "pclmulqdq":
        found.append("pclmul")
    if name == "bnd":
        found.append("mpx")
    return list(dict.fromkeys(found))


class Symbols:
    """`nm -S` ranges, so a hit can be attributed to the function around it."""

    def __init__(self, path: str) -> None:
        self.ranges: list[tuple[int, int, str]] = []
        try:
            out = subprocess.run(
                ["nm", "-n", "--defined-only", "-S", path],
                capture_output=True,
                text=True,
            ).stdout
        except FileNotFoundError:
            sys.exit("nm introuvable : installez binutils (paquet `binutils`)")
        for line in out.splitlines():
            parts = line.split(maxsplit=3)
            if len(parts) == 4:
                address, size, _kind, name = parts
            elif len(parts) == 3:
                address, _kind, name = parts
                size = "0"
            else:
                continue
            try:
                start = int(address, 16)
                length = int(size, 16)
            except ValueError:
                continue
            if length:
                self.ranges.append((start, start + length, name))
        self.ranges.sort()
        self.starts = [item[0] for item in self.ranges]

    def __bool__(self) -> bool:
        return bool(self.ranges)

    def at(self, address: int) -> str:
        index = bisect.bisect_right(self.starts, address) - 1
        if index >= 0:
            start, end, name = self.ranges[index]
            if start <= address < end:
                return name
        return ""


def inputs(paths: list[str]) -> list[str]:
    """Expand directories to the objects/archives inside (one objdump per file)."""
    found: list[str] = []
    for path in paths:
        if os.path.isdir(path):
            for root, _dirs, files in os.walk(path):
                for name in sorted(files):
                    if name.endswith((".o", ".a", ".so")):
                        found.append(os.path.join(root, name))
        else:
            found.append(path)
    return found


def scan(
    path: str,
    symbols: Symbols,
    gate: tuple[str, ...],
    samples: int,
    ours_always: bool,
    gated_set: list[str],
):
    counts = {key: 0 for key in DESCRIPTIONS}
    own = {key: 0 for key in DESCRIPTIONS}
    first: dict[str, str] = {}
    process = subprocess.Popen(
        # Plain `-d`: the raw instruction bytes are load-bearing.
        ["objdump", "-d", path],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        errors="replace",
    )
    assert process.stdout is not None
    instructions = 0
    for line in process.stdout:
        parsed = instruction(line)
        if parsed is None:
            continue
        raw, mnemonic, operands = parsed
        if mnemonic.startswith("(") or mnemonic.startswith("."):
            continue
        instructions += 1
        categories = classify(raw, mnemonic, operands)
        if not categories:
            continue
        address = int(line.split(":", 1)[0].strip(), 16)
        owner = "" if ours_always else symbols.at(address)
        ours = ours_always or any(owner.startswith(prefix) for prefix in gate)
        for category in categories:
            counts[category] += 1
            if ours:
                own[category] += 1
                if category in gated_set:
                    first.setdefault(category, f"{owner}  {line.strip()}")
    stderr = process.stderr.read() if process.stderr else ""
    code = process.wait()
    # An object with no `.text` (hbm.cpp.o is one, on x86) prints nothing and is
    # not a failure; a binary that disassembles to nothing is.
    if code != 0 or (instructions == 0 and not ours_always):
        sys.exit(f"objdump a échoué sur {path} (code {code}): {stderr.strip()[:400]}")
    return counts, own, first, instructions


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("paths", nargs="+", help="binaries, objects, or directories")
    parser.add_argument(
        "--baseline",
        default="x86-64-v2",
        help="the floor to enforce (the level of the binary being checked); a new "
        "level is a decision, with its note, its measurement and its cache key",
    )
    parser.add_argument(
        "--gate",
        default="ggml,whisper",
        help="comma-separated symbol prefixes this check gates on (the code we compile)",
    )
    parser.add_argument(
        "--report",
        action="store_true",
        help="count everything, gate nothing (a stripped binary, or a dependency we "
        "do not control): exits 0 unless the input cannot be read",
    )
    parser.add_argument("--samples", type=int, default=2)
    args = parser.parse_args()
    if args.baseline not in LEVELS:
        sys.exit(
            f"plancher inconnu : {args.baseline}. Les planchers documentés sont "
            f"{', '.join(LEVELS)} (docs/agents/desktop.md, *The CPU floor*)."
        )
    level = LEVELS[args.baseline]
    gated_set = gated_categories(level)
    gate = tuple(part.strip() for part in args.gate.split(",") if part.strip())

    failed = False
    totals = {key: 0 for key in DESCRIPTIONS}
    for path in inputs(args.paths):
        # An object file is ours from end to end and its sections restart at
        # address 0, so a symbol range cannot be trusted there: everything gates.
        # A linked binary or an archive is attributed through `nm`.
        ours_always = path.endswith((".o", ".a"))
        symbols = Symbols(path)
        counts, own, first, instructions = scan(
            path, symbols, gate, args.samples, ours_always, gated_set
        )
        for key in DESCRIPTIONS:
            totals[key] += own[key]
        gated = sum(own[key] for key in gated_set)
        print(f"{path} — {instructions} instructions, {gated} hors plancher")
        if not ours_always and not symbols and not args.report:
            print(
                "  pas de table de symboles : impossible d'attribuer les instructions.\n"
                "  Passez le binaire de débogage ou les objets (build/…/ggml/src), ou "
                "--report pour un simple relevé."
            )
            failed = True
            continue
        for category, description in DESCRIPTIONS.items():
            count = counts[category]
            mine = own[category]
            mark = "!!" if (category in gated_set and mine) else "  "
            print(f"  {mark} {category:<9} {count:>8}  {description}")
            if mine != count:
                print(f"        dont {mine} que l'on compile, {count - mine} en code dispatché (CPUID)")
            if mine and category in first:
                print(f"        {first[category][:150]}")
        if args.report:
            print(f"  relevé seulement (--report) : {gated} instructions hors plancher,"
                  f" dont {sum(totals[key] for key in gated_set)} cumulées")
        elif gated:
            failed = True
            print(f"  {path} dépasse {args.baseline} : {gated} instructions.")
        else:
            print(f"  {path} tient dans {args.baseline}.")
        print()
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
