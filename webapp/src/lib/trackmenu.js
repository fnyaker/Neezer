// ONE place builds every contextual menu of a title, an episode, an album /
// playlist / artist card or a show, and one place opens it.
//
// Why a module of its own: the same person-facing actions were being reached
// from the row's "⋯", a right-click, a long press in the Android WebView and —
// from now on — the queue, an episode row, a shelf card, a history event and a
// disliked title. The LABELS and the ACTIONS are the contract: they live here,
// once, and a surface only says which object it is about.
//
// The mutating operations still live in lib/actions.js (they own the API and
// the optimistic stores). They are handed in by `createTrackMenus` rather than
// imported, so this module has no import cycle with actions.js: the direction
// is actions.js -> trackmenu.js, always.
import { get } from "svelte/store";
import { REMOTE, CAN_KEEP, CAN_BROWSE } from "./remote/mode.js";
import {
  player,
  favorites,
  disliked,
  playlists,
  lastPlaylist,
  isAdmin,
  openShare,
  openCredits,
  openReplace,
  openGenreTag,
  openPlaylistPicker,
  openMoveToFolder,
  openMenu,
} from "./stores.js";
import { isDownloaded } from "./offline.js";
import { addMarkerAt } from "./markers.js";
import { credits } from "./format.js";
import { entityRef, trackEntityRef } from "./entity.js";
import { trackKey, trackRef } from "./trackkey.js";

// Quality choices offered in the "download as…" submenu.
export const DL_QUALITIES = [
  { id: "FLAC", label: "FLAC" },
  { id: "OPUS_320", label: "Opus 320" },
  { id: "OPUS_256", label: "Opus 256" },
  { id: "OPUS_192", label: "Opus 192" },
  { id: "OPUS_128", label: "Opus 128" },
  { id: "OPUS_64", label: "Opus 64" },
];

// What the server needs to name an entity it has never imported.
export function entityMeta(kind, item) {
  if (kind === "album")
    return {
      title: item.title || item.name,
      artist: item.artist?.name,
      artist_id: item.artist?.deezer_id,
    };
  if (kind === "artist") return { title: item.name || item.title };
  return { title: item.title || item.name };
}

// The menu's vocabulary, bound once by actions.js. `ops` holds the operations
// that write (favourite, radio, playlist, download, entity actions, podcast
// subscription) — the menu itself never calls the API, and opening it costs no
// request: it only reads stores the app already has.
export function createTrackMenus(ops) {
  // A queue row is not a place to queue the same title again. Everything else
  // it can do stays; removing is the queue's own verb, and only for a title
  // that is still ahead of the playhead (what the queue's store allows).
  function removeFromQueue(track) {
    const s = get(player);
    const key = trackKey(track);
    for (let i = s.index + 1; i < s.queue.length; i++) {
      if (trackKey(s.queue[i]) === key) {
        player.removeAt(i);
        return;
      }
    }
  }

  // A podcast episode is shaped like a track (so the queue/player play it), but
  // its id is a UUID and its "artist"/"album" are the channel — so the normal
  // track menu's favorite / radio / playlist / go-to-artist actions all hit
  // endpoints that reject a UUID (400) or 404. Give episodes their own minimal,
  // working menu instead.
  function buildEpisodeMenu(ep, nav, options = {}) {
    const inQueue = options.surface === "queue";
    const dl = isDownloaded(trackKey(ep));
    const items = [];
    if (inQueue) {
      if (Number.isInteger(options.index) && options.index > (options.currentIndex ?? -1))
        items.push({
          label: "Retirer de la file",
          icon: "close",
          action: () => removeFromQueue(ep),
        });
    } else {
      items.push(
        { label: "Lire ensuite", icon: "next", action: () => player.playNext([ep]) },
        { label: "Ajouter à la file", icon: "queue", action: () => player.addToQueue([ep]) }
      );
    }
    // Playing this very episode: offer a one-tap marker at the current position.
    const s = get(player);
    if (trackKey(s.queue[s.index]) === trackKey(ep)) {
      items.push({
        label: "Marquer cette position",
        icon: "bookmarkPlus",
        action: () => addMarkerAt(ep, get(player).currentTime),
      });
    }
    items.push(
      { label: "Partager…", icon: "share", action: () => openShare(ep) },
      "divider",
      dl
        ? {
            label: "Retirer le téléchargement",
            icon: "downloaded",
            action: () => ops.undownloadTrack(ep),
          }
        : { label: "Télécharger", icon: "download", action: () => ops.downloadTrackTo(ep) }
    );
    if (ep.channel_id)
      items.push("divider", {
        label: "Ouvrir le podcast",
        icon: "mic",
        action: () => nav("/podcast/" + ep.channel_id),
      });
    return items;
  }

  // Build the context-menu item list for a track. `nav` is svelte-spa-router push.
  // `options.surface === "queue"` with `index` / `currentIndex` makes it the
  // queue's own menu (no "queue this again", plus "remove from the queue").
  function buildTrackMenu(track, nav, options = {}) {
    if (track.podcast) return buildEpisodeMenu(track, nav, options);
    const fav = get(favorites).has(trackKey(track));
    // The owner's identity, narrowed on a remote control to what its link lends.
    const admin = get(isAdmin) && CAN_KEEP;
    const inQueue = options.surface === "queue";
    const items = [];
    if (inQueue) {
      if (Number.isInteger(options.index) && options.index > (options.currentIndex ?? -1))
        items.push({
          label: "Retirer de la file",
          icon: "close",
          action: () => removeFromQueue(track),
        });
    } else {
      items.push(
        { label: "Lire ensuite", icon: "next", action: () => player.playNext([track]) },
        { label: "Ajouter à la file", icon: "queue", action: () => player.addToQueue([track]) }
      );
    }
    // Adding to a playlist is every account's business now: a playlist shared
    // with an editor is a legitimate target, so these entries follow CAN_KEEP
    // instead of the admin flag. The one-tap shortcut only shows when the store
    // says that playlist is writable — a viewer's list is not a target.
    // `trackRef` names the track the way the server resolves it: the row UUID, or
    // the Deezer id of a catalogue result the playlist will make a row for.
    if (CAN_KEEP) {
      const last = get(lastPlaylist);
      const entry =
        last && last.id
          ? (get(playlists) || []).find((p) => String(p.id) === String(last.id))
          : null;
      if (entry && entry.editable)
        items.push({
          label: `Ajouter à « ${last.title} »`,
          icon: "plus",
          action: () => ops.addTrackToPlaylist(last.id, trackRef(track), last.title),
        });
      // Opens the searchable playlist picker (scrolling a giant submenu to find
      // the right playlist was the old, painful way).
      items.push({
        label: "Ajouter à une playlist…",
        icon: "music",
        action: () => openPlaylistPicker(track),
      });
    }
    if (admin) {
      // Swap this track for another one everywhere it appears. Always offered —
      // a track can also be simply *wrong* (bad master, wrong version), not only
      // dead — but it leads the list when we already know it's unplayable.
      items.push({
        label: "Remplacer le titre…",
        icon: "refresh",
        action: () => openReplace(track),
      });
      // Tagging from wherever the track is. The studio is where you tag a library
      // in one sitting; this is for the other moment — you are looking at a list,
      // you see a genre that is wrong, and fixing it should not mean finding the
      // track again somewhere else. The sheet opens OVER the current screen, so
      // nothing is lost by using it.
      items.push({
        label: "Étiqueter le genre…",
        icon: "tag",
        action: () => openGenreTag(track),
      });
    }
    // Offline download — device-local, so available to everyone. A plain
    // "Télécharger" at the default quality, plus a submenu to pick another. Not on
    // a remote control: this device is not the one playing.
    const dl = !REMOTE && isDownloaded(trackKey(track));
    if (!REMOTE)
      items.push(
        "divider",
        dl
          ? {
              label: "Retirer le téléchargement",
              icon: "downloaded",
              action: () => ops.undownloadTrack(track),
            }
          : { label: "Télécharger", icon: "download", action: () => ops.downloadTrackTo(track) }
      );
    if (!dl && !REMOTE)
      items.push({
        label: "Télécharger en…",
        icon: "download",
        sub: DL_QUALITIES.map((q) => ({
          label: q.label,
          icon: "download",
          action: () => ops.downloadTrackTo(track, q.id),
        })),
      });

    items.push("divider");
    if (CAN_KEEP) items.push({ label: "Partager…", icon: "share", action: () => openShare(track) });
    items.push({
      label: "Lancer la radio",
      icon: "radio",
      action: () => ops.startTrackRadio(track),
    });
    if (CAN_KEEP)
      items.push({
        label: fav ? "Retirer des favoris" : "Ajouter aux favoris",
        icon: fav ? "heartFilled" : "heart",
        action: () => ops.toggleFavorite(track),
      });
    if (CAN_KEEP) {
      const no = get(disliked).has(trackKey(track));
      items.push({
        label: no ? "Retirer de « Je n'aime pas »" : "Je n'aime pas",
        icon: no ? "thumbsDownFilled" : "thumbsDown",
        action: () => ops.toggleDislike(track),
      });
    }
    items.push("divider");
    // Who wrote, composed and produced it — over the current screen.
    if (CAN_BROWSE)
      items.push({ label: "Voir les crédits", icon: "users", action: () => openCredits(track) });
    // One entry per credited artist. A single credit stays a plain item (the
    // common case shouldn't grow a submenu for nothing); a feat. track opens a
    // submenu so the guest is reachable too.
    const people = credits(track).filter((a) => entityRef(a));
    if (people.length === 1)
      items.push({
        label: "Aller à l'artiste",
        icon: "user",
        action: () => nav("/artist/" + trackEntityRef(track, "artist", people[0])),
      });
    else if (people.length > 1)
      items.push({
        label: "Aller à l'artiste",
        icon: "user",
        sub: people.map((a) => ({
          label: a.name,
          icon: "user",
          action: () => nav("/artist/" + trackEntityRef(track, "artist", a)),
        })),
      });
    if (entityRef(track.album))
      items.push({
        label: "Aller à l'album",
        icon: "disc",
        action: () => nav("/album/" + trackEntityRef(track, "album", track.album)),
      });
    return items;
  }

  // Context menu for an album/artist/playlist card.
  function buildEntityMenu(kind, item, nav) {
    const route = "/" + kind + "/";
    const routeId = entityRef(item);
    const items = [
      { label: "Lire", icon: "play", action: () => ops.playEntity(kind, routeId) },
      { label: "Ouvrir", icon: "open", action: () => nav(route + routeId) },
    ];
    if (kind === "playlist" && CAN_KEEP) {
      const mine = (get(playlists) || []).find((p) => String(p.id) === String(item.id));
      if (mine) {
        items.push({
          label: "Déplacer vers un dossier…",
          icon: "folderMove",
          action: () => openMoveToFolder({ playlist: mine }),
        });
        if (mine.folder_id)
          items.push({
            label: "Retirer du dossier",
            icon: "folderMinus",
            action: () => ops.movePlaylistTo(mine.id, null),
          });
      }
    }
    if ((kind === "album" || kind === "playlist") && !REMOTE)
      items.push({
        label: "Télécharger",
        icon: "download",
        action: () => ops.downloadEntity(kind, routeId),
      });
    // A favourite is local to the user's own account; on a remote control, only a
    // link that may change what is kept can make one.
    if (CAN_KEEP) {
      const meta = entityMeta(kind, item);
      if ((kind === "album" || kind === "playlist") && item.deezer_id)
        items.push({
          label: "Ajouter aux favoris",
          icon: "heart",
          action: () => ops.toggleEntityFavorite(kind, routeId, true, meta),
        });
      if (kind === "artist" && routeId)
        items.push({
          label: "Suivre",
          icon: "heart",
          action: () => ops.toggleEntityFavorite("artist", routeId, true, meta),
        });
    }
    return items;
  }

  // A show reached from a search result has no local row yet: subscribing is
  // what makes it readable, and the server imports it on its first play. Two
  // honest actions — open it now, or keep it for later without leaving the
  // results. A subscribed show (its row id exists) opens directly.
  function buildPodcastMenu(item, nav, options = {}) {
    const id = entityRef(item);
    if (options.subscribed)
      return [
        { label: "Ouvrir", icon: "open", action: () => nav("/podcast/" + id) },
      ];
    return [
      {
        label: "S'abonner et ouvrir",
        icon: "plus",
        action: () => {
          ops.subscribeToPodcast(item.deezer_id).then((c) => {
            if (c?.id) nav("/podcast/" + c.id);
          });
        },
      },
      {
        label: "S'abonner",
        icon: "mic",
        action: () => {
          ops.subscribeToPodcast(item.deezer_id);
        },
      },
    ];
  }

  // `trackRef` names the row the server will resolve (a library UUID or a
  // catalogue id); the menu reads it from trackkey.js, not actions.js.
  return {
    track: buildTrackMenu,
    episode: buildEpisodeMenu,
    entity: buildEntityMenu,
    podcast: buildPodcastMenu,
  };
}

// -- opening it --------------------------------------------------------------

// Shift+F10 and the keyboard's Menu key are the two the ARIA practices name for
// "open this element's context menu".
export function isMenuKey(event) {
  return !!event && (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10"));
}

// Where the menu should appear: under the pointer, or against the trigger for a
// keyboard call (clientX/Y are 0 then).
export function menuAnchor(event, trigger) {
  const x = Number(event?.clientX) || 0;
  const y = Number(event?.clientY) || 0;
  if (x || y) return { x, y };
  const box = trigger?.getBoundingClientRect?.();
  if (!box) return { x: 0, y: 0 };
  return { x: Math.round(box.left + 12), y: Math.round(box.bottom + 4) };
}

// A right-click on text somebody had selected asked for the browser's Copy
// menu, not ours. Same for a field: its own menu is the useful one. Shift+F10 /
// the Menu key is an explicit request for OUR menu, even with a selection up.
export function keepNativeMenu(event) {
  if (event?.type === "keydown") return false;
  const el = event?.target;
  if (el?.closest?.('input, textarea, select, [contenteditable=""], [contenteditable="true"]')) return true;
  return selectionInside(event?.currentTarget);
}

function selectionInside(node) {
  try {
    const sel = window.getSelection?.();
    if (!sel || sel.isCollapsed || !sel.rangeCount || !node) return false;
    return !!sel.anchorNode && node.contains(sel.anchorNode);
  } catch {
    return false;
  }
}

// Open `items` at the event. The trigger is handed to openMenu as the event's
// currentTarget, which is what focuses it (so Escape can give the focus back);
// a keyboard call has already focused it and is left alone.
export function openMenuAt(event, items, trigger) {
  if (!items || !items.length) return;
  const node = trigger || event?.currentTarget || null;
  const at = menuAnchor(event, node);
  openMenu(
    {
      clientX: at.x,
      clientY: at.y,
      currentTarget: node,
      preventDefault: () => event?.preventDefault?.(),
      stopPropagation: () => event?.stopPropagation?.(),
    },
    items
  );
}

// Shift+F10 / Menu on the focused row. Returns true when it handled the key, so
// a caller can keep its own Enter/Space behaviour in one handler.
export function menuKey(event, open) {
  if (!isMenuKey(event)) return false;
  event.preventDefault();
  open(event);
  return true;
}
