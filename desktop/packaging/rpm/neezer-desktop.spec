# The RPM half of the desktop app's packaging, built by `build.sh` from the
# release binaries (one build for four formats; see ../lib.sh).
#
# Why a spec and `rpmbuild` rather than `cargo-generate-rpm`
# (https://github.com/cat-in-136/cargo-generate-rpm):
#
#   - `cargo-generate-rpm` does NOT build — it packages what `cargo build`
#     already put in `target/release/`, which is exactly this job's shape — but
#     it reads its metadata from `[package.metadata.generate-rpm]` in ONE crate's
#     `Cargo.toml`, and it needs a `cargo install` (a Rust toolchain, a crate
#     registry, a version to pin) in the packaging job. A spec is a text file the
#     Fedora container's own `rpmbuild` reads, with no tool to install and no
#     manifest to edit.
#   - `rpmbuild` derives `Requires` from the two ELF files' NEEDED entries by
#     itself (`libpipewire-0.3.so.0()(64bit)`, `libvulkan.so.1()(64bit)`, …),
#     which dnf resolves to the right package names on every RPM distribution:
#     the equivalent of the Debian table in `../lib.sh`, done by the distro's own
#     generator. `Requires: AutoReqProv: yes` is the default and must stay.
%global debug_package %{nil}

Name:           neezer-desktop
Version:        %{neezer_version}
Release:        1
Summary:        Lecteur de musique et de podcasts Neezer (GPUI, PipeWire, MPRIS)
License:        AGPL-3.0-or-later
URL:            https://github.com/fnyaker/Neezer
ExclusiveArch:  x86_64

# The launcher finds `neezer-desktop.svg` through the hicolor hierarchy this
# package owns (namcap's `E:` was what noticed on Arch, P1).
Requires:       hicolor-icon-theme
# The runtime dependencies of the two binaries are NOT listed here: rpmbuild
# generates them from the binaries (see the header). What it cannot generate is
# what is not a shared library.
Recommends:     pipewire
Recommends:     pipewire-pulseaudio
Recommends:     mesa-vulkan-drivers
# GPUI resolves a sans font at startup and aborts when fontconfig has none; a
# desktop always has one, a bare container does not (deb/control.in has the same
# note and the measured message).
Recommends:     dejavu-sans-fonts

%description
Lecteur de musique et de podcasts pour les bureaux Linux : interface GPUI,
lecture gapless et crossfade par PipeWire, MPRIS (les touches multimédia et la
barre du bureau pilotent le lecteur), cache hors ligne, et transcription locale
des épisodes sur le GPU via Vulkan.

Music and podcast player for Linux desktops: GPUI interface, gapless and
crossfaded playback through PipeWire, MPRIS (the media keys and the desktop's
player widget drive it), an offline cache, and local podcast transcription on
the GPU through Vulkan.

Both binaries are installed: neezer-desktop (the app) and neezer-transcribe (the
sidecar it starts, whisper.cpp with the Vulkan backend; it falls back to the CPU
and says why when no Vulkan device is usable).

%install
rm -rf %{buildroot}
install -Dpm 755 %{_neezer_stage}/neezer-desktop %{buildroot}%{_bindir}/neezer-desktop
install -Dpm 755 %{_neezer_stage}/neezer-transcribe %{buildroot}%{_bindir}/neezer-transcribe
install -Dpm 644 %{_neezer_stage}/neezer-desktop.desktop \
  %{buildroot}%{_datadir}/applications/neezer-desktop.desktop
install -Dpm 644 %{_neezer_stage}/neezer-desktop.svg \
  %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/neezer-desktop.svg
install -Dpm 644 %{_neezer_stage}/LICENSE \
  %{buildroot}%{_licensedir}/%{name}/LICENSE

%files
%{_bindir}/neezer-desktop
%{_bindir}/neezer-transcribe
%{_datadir}/applications/neezer-desktop.desktop
%{_datadir}/icons/hicolor/scalable/apps/neezer-desktop.svg
%license %{_licensedir}/%{name}/LICENSE

# No `%check`: the workspace's suite starts the Python server and runs an audio
# soak (the Arch package skips it for the same reason, P1). The `test` job of
# desktop.yaml owns it; `test.sh` here starts the installed binary for real.
