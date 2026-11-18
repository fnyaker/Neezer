# `mobile/brand` — the logo's sources

The launcher icon, its monochrome (themed) twin and the notification glyph are
generated from the files here; the app only ever references the *names*
(`ic_launcher`, `ic_launcher_round`, `ic_stat_neezer`), so swapping the artwork
never touches a manifest, a Gradle file or a Kotlin file.

**Lot LG1 owns this directory** and the launcher resources the ownership table
names (`drawable*/ic_launcher*`, `drawable/ic_stat_neezer.xml`,
`mipmap-anydpi-v26/ic_launcher*.xml`).

## The mark: « La lentille »

A disc, and its lens laid off-centre: the gorge around the lens is the groove,
the lens is the light. Nothing is concentric, nothing sits at the centre — that
is Sillon's own off-centre rhythm (the missing-cover groove is centred on
68 % / 32 %).

The icon is **not a fixed black tile**: it follows the theme, because Sillon's
rule is that *a theme is a colour pair*. Same geometry, two arrangements:

| file | theme | ground | disc | gorge | lens |
|---|---|---|---|---|---|
| `logo.svg` | Encre (night, primary) | ink `#0c0c0b` | lemon `#d2ff3c` | a **hole**: the ink shows | cream `#f3f0e8` |
| `logo-papier.svg` | Papier (day) | cream `#f3f0e8` | ink `#0c0c0b` | **painted lemon** (Papier's highlighter) | ink `#0c0c0b` |
| `logo-mono.svg` | — | — | the mask, one colour | a hole | a separate shape |
| `logo-notification.svg` | — | — | 24 dp silhouette | — | — |

Papier paints the gorge lemon instead of leaving it a hole for a measured
reason: a hole in Papier would be *cream*, and cream against the lemon lens is
1.05:1 — the lens would lose its edge (a hue-only edge, invisible in
greyscale). Painted lemon, every edge of the Papier mark is ≥ 16.9:1 and the
highlighter stays.

`logo-mono.svg` is the **same geometry in one colour**: it is the mask the
launcher tints for themed icons (Android 13+), so the themed icon is not a
redrawing. The notification glyph is the same silhouette at an **optical size**
of 24 dp: nothing under 2 dp, so its gorge is 3 dp and its lens 6.4 dp across.

## The three directions the person chose from

`directions/` keeps the frozen candidates — not sources to keep in sync, the
record of the choice:

| file | name | structure |
|---|---|---|
| `directions/a-lens.svg` | La lentille | the disc and its off-centre lens — **retained** |
| `directions/b-sleeve.svg` | La pochette | a lemon groove under a cream sleeve; the disc reads through it |
| `directions/c-wake.svg` | Le sillage | two furrows tapering to a point; no disc at all |

Each has its `*-mono.svg` beside it. What the mark may not be: the old SPA mark
(a lemon disc with two concentric circles and a centre dot) or anyone else's —
Spotify's three arcs, Deezer's bars and heart, Apple Music's note, YouTube
Music's play triangle in a disc, Tidal's diamonds, Qobuz's Q. No text, no emoji,
no gradient that dies in monochrome.

## What the palette forces (measured, not a taste)

- **Cream and lemon have the same luminance** — `#f3f0e8` is 1.14:1 against
  white, `#d2ff3c` 1.16:1, and they are **1.02:1 against each other**. A cream
  shape on a lemon one, or the reverse, is a *hue-only* edge: legible on a
  screen, gone in greyscale. So no cream shape ever touches a lemon one in
  either variant (measured: 5.60 dp apart in Encre, 0 in Papier because Papier
  paints the gorge lemon *inside the ink disc*, where it meets only ink).
- **At 16 px only two tones survive.** 1 px = 6.75 dp on the 108 dp canvas, so
  any detail under 6.75 dp is sub-pixel at the smallest launcher size. A spiral
  groove tried first fragmented the disc into 1–2 px specks at 16 px (five
  components instead of two, measured) and was dropped: the mark keeps two
  shapes at 16 px.
- **The safe zone is a circle, not a box.** The 66 dp area no mask clips is the
  circle of radius 33 dp around (54, 54); a 60 dp disc has a 60×60 box whose
  corners are 42 dp out, so a box-corner test would reject every round mark.
  `check.py` measures the *pixels*.

## `check.py` — the acceptance, measured

```sh
python3 mobile/brand/check.py            # logo.svg + logo-papier.svg + mono + glyph
```

No network; needs `rsvg-convert`, ImageMagick and numpy. It fails on:

1. **Safe zone** — the farthest opaque pixel of the foreground layer, in dp,
   against 33 dp (1080 px raster = 10 px per dp), for **both** themes.
2. **Every touching edge ≥ 3:1** — measured on the render, not guessed from a
   colour list: the script finds which colour pairs actually touch and computes
   their WCAG ratio. That is what caught the Papier lens sitting in a cream
   well at 1.02:1. Measured: Encre's worst edge 16.88:1, Papier's 16.88:1.
3. **Reads on both wallpapers** — for each theme, on a white and on a black
   wallpaper, at least one of its shapes holds ≥ 3:1: Encre on white 19.57:1
   (the ink tile), Papier on black 17.18:1 (the cream tile); the other way
   round the mark itself carries (18.11:1 and 19.57:1).
4. **Silhouette at 16 px** — connected components of the monochrome layer, and
   each component's average width (`area / longest side`), which must stay
   ≥ 1 px: that is "no detail below 1 px". Plus a 2×2 opening as a guard.
   Measured: 2 shapes, widths 5.75 and 2.0 px.
5. **Glyph at 24 dp** — where 1 px = 1 dp, a 2×2 opening must keep ≥ 90 % of the
   area. Measured: a 1 dp band falls to **0 %**, a 2 dp band keeps 94–100 %; the
   glyph measures 98 %.
6. **Monochrome** — the mask's shape count at 16 px (2) and its contrast in the
   two themed modes that exist (17.2:1 each). White-on-white is not a mode.

## Export

```sh
mobile/brand/export.sh            # write every generated resource
mobile/brand/export.sh --check    # fail if one of them has drifted from the sources
```

Offline, stdlib Python + `rsvg-convert` (or `resvg`); no Gradle, no network. It
writes:

- `drawable/ic_launcher_background.xml` and `drawable-night/...` — the Papier
  and Encre grounds;
- `drawable/ic_launcher_foreground.xml` and `drawable-night/...` — the Papier
  and Encre marks;
- `drawable/ic_launcher_monochrome.xml` — the mask, theme-independent;
- `mipmap-anydpi-v26/ic_launcher.xml` + `ic_launcher_round.xml` — the
  `<adaptive-icon>`, whose `<background>` points at
  `@drawable/ic_launcher_background`, so the day/night pair lives entirely in
  `drawable*/` and **no shared file is touched** (`values/colors.xml` belongs to
  MB0 and is never written);
- `drawable/ic_stat_neezer.xml` — the 24 dp white status-bar silhouette;
- `play-store-512.png` and `play-store-512-papier.png` — 512×512, 32-bit, sRGB,
  opaque, full square (Play applies the 30 % radius and the shadow itself);
- `icon-encre-108.png`, `icon-papier-108.png` — the two composed icons, for docs.

Three rules the script enforces instead of documenting:

- **An element it does not understand is a hard error** — a silently dropped
  element is a wrong icon, and a wrong icon ships. Supported: `<path>`,
  `<circle>`, `<ellipse>`, `<rect>` (as `pathData`), `fill-rule` `evenodd` (as
  `android:fillType`), strokes, and `rotate(a cx cy)` as a `<group>`.
- **`--check` is the drift guard**: it renders into a temp tree and compares
  byte for byte. Tested in both directions (matching, and one injected comment
  in a committed drawable).
- **`NZ_RES` / `NZ_BRAND_OUT`** redirect the output, so the tests export into a
  scratch tree and the repository never carries a scratch render.

Checked against `aapt2 compile` + `aapt2 link` (a minimal manifest,
`--min-sdk-version 26`) for all three directions and for both themes: every
generated drawable links.

## Evidence on the emulator

The lot's emulator image (API 35 `default`, AVD `nz35`) ships **neither** the
`com.android.theme.icon.*` overlays **nor** the themed-icons setting: `cmd
overlay list` has one `com.android.theme.font.*` entry and nothing else, and
`settings list secure` has no `themed_icons`. The launcher can only ever show
its own mask, so two things are captured, both headless (`-no-window`
`-no-audio`, `adb -s emulator-<port>` only, the lot's own PID):

1. **The real launcher**, in both themes: `adb shell cmd uimode night no|yes`,
   the icon in the app drawer, `adb exec-out screencap`. This is what proves the
   light version, since the launcher picks the `-night` resources itself.
2. **The three masks, the themed icon and the glyph**, drawn *on the device* by
   the framework's own `AdaptiveIconDrawable` over the **installed APK's real
   resources** (`AssetManager.addAssetPath` + `getIdentifier`), applying the AOSP
   mask geometry (circle, superellipse squircle, teardrop with its sharp corner),
   the monochrome layer tinted in the two modes that exist, and the 24 dp glyph
   at this screen's density (420 dpi → 63 px) and four times larger — where a
   stroke under 2 dp would show:

```sh
mobile/brand/device/render-masks.sh --serial emulator-5600 \
    --apk mobile/android/app/build/outputs/apk/debug/app-debug.apk \
    --out ~/.cache/neezer-lg1/shots
```

It compiles `device/IconMasks.java` with the platform's `d8`, pushes the dex,
runs it as `app_process` under the shell uid (no app, no signature, no Gradle),
pulls the PNGs and writes strips.

`mobile/tools/emu.sh` (lot MQ1) owns the emulator lifecycle; this lot's port is
**5600** (5580 belongs to MQ1). Before that harness lands, the same flags are
used by hand: `emulator -avd nz35 -port 5600 -no-window -no-audio -gpu
swiftshader_indirect -no-boot-anim -no-snapshot-save -readonly -memory 3072
-cores 4`.
