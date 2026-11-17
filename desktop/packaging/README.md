# Packaging the desktop app

Five formats, one app, one release build. `.github/workflows/release.yaml`
attaches one file per format to every `v*` release, and each job installs and
starts what it built before attaching it.

| Format | File | Installed by | Built with | Job |
|---|---|---|---|---|
| Arch | `neezer-desktop-<ver>-x86_64.pkg.tar.zst` | `pacman -U` | `makepkg` | `arch` |
| Debian/Ubuntu | `neezer-desktop-<ver>-x86_64.deb` | `apt install ./…` | `dpkg-deb` | `deb` |
| Fedora/RHEL | `neezer-desktop-<ver>-x86_64.rpm` | `dnf install ./…` | `rpmbuild` | `rpm` |
| any desktop | `neezer-desktop-<ver>-x86_64.AppImage` | `chmod +x` then run it | `appimagetool` | `appimage` |
| any desktop | `neezer-desktop-<ver>-x86_64.flatpak` | `flatpak install --user ./…` | `flatpak-builder` | `flatpak` |

Every one of them installs **both** binaries — `neezer-desktop` and its sidecar
`neezer-transcribe` — in the same directory, because that is where the app looks
for it (`app/src/wiring/transcribe.rs#program` resolves the running executable's
own directory before `PATH`). A package that installs only the app transcribes
nothing and says « Le binaire neezer-transcribe est introuvable ».

## Building one locally

From the repository root, with a release build of the workspace:

```sh
cd desktop && cargo build --release --workspace && cd ..
export NEEZER_DESKTOP_VERSION=1.6.0          # what a tag would pass (v1.6.0 -> 1.6.0)
desktop/packaging/deb/build.sh release/       # or rpm/, appimage/, flatpak/
desktop/packaging/rpm/test.sh release/neezer-desktop-1.6.0-x86_64.rpm
```

`desktop/packaging/version.sh` prints the version a package would carry: the
tag's when the build is a tag, the workspace's `0.1.0` otherwise.

A local build is only as portable as the machine that built it: the `.deb`
declares the glibc floor **measured** from the binary (`libc6 (>= 2.39)` for the
release's Ubuntu 24.04 build), so a package built on this Arch machine says what
its own binaries need, which no current release distribution has. **The release
artefacts come from CI**, where the same binary is built once on the oldest base
we support and every package reuses it.

## What the rule is, and why

- **`depends` follow the binaries, not the plan** (P1's lesson: `namcap`'s `E:`
  lines were the missing `libxcb` and `libxkbcommon-x11`). `lib.sh` holds one
  table of the sonames the two binaries are allowed to need, and
  `neezer_check_needed` reads `readelf -d` and **fails the build** of every format
  on a soname that is not in it. The Debian `Depends:` line is generated from
  that same table; `rpmbuild` derives its own from the same sonames (which is why
  the RPM lists no dependencies by hand).
- **Vulkan comes from the machine.** `neezer-transcribe` links `libvulkan.so.1`
  and enumerates devices at startup (docs/agents/desktop.md, T2); the loader, the
  ICDs and the Mesa drivers are the person's, in every format. The AppImage
  bundles none of it and `appimage/test.sh` fails if a `libvulkan*` ever appears
  inside; the Flatpak gets it from the runtime's `GL.default` extension through
  `--device=dri`.
- **One launcher entry, one icon.** `arch/neezer-desktop.desktop` and
  `arch/neezer-desktop.svg` are installed by every format; the Flatpak renames
  them to its app id (`rename-desktop-file`/`rename-icon`/`copy-icon`) and rewrites
  the single `StartupWMClass` line, because inside the sandbox the window's
  app id is the sandbox's (`app/src/main.rs#flatpak_app_id`).

## What each format is for

- **Arch** — the person this app was written for: Omarchy, Hyprland, PipeWire,
  `pacman -U` and the package is on the launcher. Built by `makepkg` in an Arch
  container from the checkout, `namcap`-checked (P1).
- **Debian** — Debian 13+ and Ubuntu 24.04+: `libpipewire-0.3-0t64` and
  `libasound2t64` on trixie, the pre-`t64` names on bookworm, hence the
  alternatives in `lib.sh`.
- **RPM** — Fedora and the RHEL family: `rpmbuild`'s own soname `Requires`.
- **AppImage** — one executable file, no install, no root: the format for a
  machine where nothing is packaged. It carries the app, not the graphics stack
  (`appimage/build.sh` explains why bundling `libwayland`/`libxkbcommon` breaks
  the next distribution).
- **Flatpak** — the sandboxed one: a runtime that owns the C library and the
  graphics stack, `--device=dri` for the GPU, `--socket=pulseaudio` for sound,
  and no filesystem permission at all because the app opens none of the person's
  files (`flatpak/fr.neezer.Desktop.yaml` lists every grant with its reason).

## Testing a package

Each format has a `test.sh` that installs (or runs) it and then starts the app on
**its own X server** (`lib.sh#neezer_xvfb_start`: a free display number from 140,
never the person's session) with `--self-test-quit-after-first-frame`, which
prints `first_frame_ms=<n>` and quits. `neezer-transcribe --version` runs the
installed sidecar and reports the whisper.cpp build and the Vulkan device it
found. The jobs in `release.yaml` run exactly these scripts in the distribution
they target:

```sh
sudo apt install ./neezer-desktop-1.6.0-x86_64.deb   # deb/test.sh
sudo dnf install ./neezer-desktop-1.6.0-x86_64.rpm   # rpm/test.sh
./neezer-desktop-1.6.0-x86_64.AppImage               # appimage/test.sh
flatpak install --user ./neezer-desktop-1.6.0-x86_64.flatpak   # flatpak/test.sh
```

## Measured

Sizes of the 1.6.0 artefacts (x86_64, both binaries included):

| Format | Size | Installed and started in |
|---|---|---|
| Arch `.pkg.tar.zst` | 26.72 MiB (P1, 1.3.1) | Arch, `pacman -U` |
| Debian `.deb` | 22 835 380 B (21.8 MiB) | Debian 13, `apt install ./…` |
| RPM | 20 238 109 B (19.3 MiB) | Fedora 45, `dnf install ./…` |
| AppImage | 32 733 688 B (31.2 MiB) | any desktop, run it |
| Flatpak | 21 980 488 B (21.0 MiB) | `flatpak --user install` |

Both binaries weigh 60 437 392 B and 38 692 288 B (99 MB installed). First frame
on the test's own X server: `.deb` 125 ms, `.rpm` 131 ms, AppImage 373 ms,
Flatpak 793 ms.

`docs/agents/desktop.md` (P2) has the rules, the numbers and the pitfalls found by
running them.
