# `mobile/brand` — the logo's sources

The launcher icon, its monochrome (themed) twin and the notification glyph are
generated from the files here; the app only ever references the *names*
(`ic_launcher`, `ic_launcher_round`, `ic_stat_neezer`), so swapping the artwork
never touches a manifest, a Gradle file or a Kotlin file.

**MB0 ships a placeholder on purpose** (a plain lemon disc: `icon.svg`), wired the
whole way through — adaptive icon foreground, monochrome layer and status-bar
glyph — so the skeleton installs with a real icon and the launcher never falls
back to a generic one.

**Lot LG1 owns this directory**: it replaces `icon.svg` with the real mark, adds
the export script, and rewrites the three drawables in
`android/app/src/main/res/` (the only files of that module LG1 may touch).
