#!/usr/bin/env sh
# Frame times on the REAL display — and this script launches NOTHING.
#
# Under Xvfb the app renders through `lavapipe` (Mesa's software Vulkan), so the
# present rate plateaus at 24–30 fps whatever the app does; that number measures
# the rasteriser, not the app. The real frame time is the person's panel. This
# script only says what to run and reads the log afterwards: it never starts a
# window, never injects a key, never touches a session.
#
#   desktop/tools/journey/frame_bench.sh            (instructions + analysis)
#   desktop/tools/journey/frame_bench.sh LOG        (analyse that log)
#
# The command to run FIRST, in a terminal, by the person (exact):
#
#   desktop/target/release/neezer-desktop \
#       --window-size 2560x1600 --frame-log /tmp/neezer-frames.log
#
# (Build it once with: cd desktop && cargo build --release -p neezer-desktop.)
# Then, with the window open, three phases of 30 seconds each:
#
#   1. NAVIGATE   — `g h`, `g l`, `g s`, `g n` in a loop, about one per second.
#   2. SCROLL     — hold `Page suivante` in the 4 000-row favourites list.
#   3. VISUALISER — press `v`, and leave the bars on screen.
#
# Quit with Ctrl+Q, then run this script again with the log path to print the
# table (p50 / p95 / max interval and fps per phase).
set -eu

LOG="${1:-/tmp/neezer-frames.log}"

if [ ! -f "$LOG" ]; then
    cat <<'TXT'
Aucun journal de mesures pour l'instant. Lancez d'abord, dans un terminal :

    desktop/target/release/neezer-desktop \
        --window-size 2560x1600 --frame-log /tmp/neezer-frames.log

Puis, dans la fenêtre, trois phases de 30 secondes :
  1. navigation  — g h, g l, g s, g n en boucle (une par seconde)
  2. défilement  — Page suivante dans Favoris (4 000 titres)
  3. visualiseur — touche v

Quittez avec Ctrl+Q, puis relancez ce script avec le chemin du journal.
TXT
    exit 0
fi

python3 "$(dirname "$0")/frame_stats.py" "$LOG" --phases 8:38,45:75,82:112
