const key = (value) => (value == null ? null : String(value));

export function groupPlaylists(playlists, folders) {
  const groups = (folders || []).map((folder) => ({
    id: key(folder.id),
    title: folder.title || "",
    position: folder.position ?? 0,
    playlists: [],
    count: 0,
  }));
  const byId = new Map(groups.map((group) => [group.id, group]));
  const unfiled = [];
  const imported = [];
  for (const playlist of playlists || []) {
    if (!playlist) continue;
    const folderId = key(playlist.folder_id);
    const group = folderId ? byId.get(folderId) : null;
    if (group) group.playlists.push(playlist);
    else if (!folderId && playlist.deezer_id) imported.push(playlist);
    else unfiled.push(playlist);
  }
  for (const group of groups) group.count = group.playlists.length;
  return { groups, unfiled, imported };
}

export function moveItem(list, from, to) {
  const source = list || [];
  const n = source.length;
  if (!n) return [];
  const f = Math.max(0, Math.min(n - 1, Number.isFinite(from) ? Math.trunc(from) : 0));
  const t = Math.max(0, Math.min(n - 1, Number.isFinite(to) ? Math.trunc(to) : 0));
  const next = source.slice();
  if (f === t) return next;
  const [item] = next.splice(f, 1);
  next.splice(t, 0, item);
  return next;
}

export function insertAt(list, item, index) {
  const next = (list || []).slice();
  const at = Math.max(0, Math.min(next.length, Number.isFinite(index) ? Math.trunc(index) : next.length));
  next.splice(at, 0, item);
  return next;
}

export function idsOf(rows) {
  return (rows || []).map((row) => key(row?.id)).filter(Boolean);
}

export function orderFolders(folders, ids) {
  const rank = new Map((ids || []).map((id, i) => [key(id), i]));
  return (folders || [])
    .map((folder, i) => ({ folder, i, r: rank.has(key(folder?.id)) ? rank.get(key(folder.id)) : Infinity }))
    .sort((a, b) => a.r - b.r || a.i - b.i)
    .map((entry) => entry.folder);
}

function containerOf(playlist, known) {
  const folderId = key(playlist?.folder_id);
  return folderId && known.has(folderId) ? folderId : null;
}

export function orderContainer(playlists, folders, folderId, ids) {
  const list = playlists || [];
  const known = new Set((folders || []).map((folder) => key(folder.id)));
  const target = key(folderId);
  const buckets = new Map();
  const push = (container, playlist) => {
    const bucket = buckets.get(container);
    if (bucket) bucket.push(playlist);
    else buckets.set(container, [playlist]);
  };
  for (const playlist of list) push(containerOf(playlist, known), playlist);

  const rank = new Map((ids || []).map((id, i) => [key(id), i]));
  const bucket = buckets.get(target) || [];
  const ordered = bucket
    .map((playlist, i) => ({
      playlist,
      i,
      r: rank.has(key(playlist?.id)) ? rank.get(key(playlist.id)) : Infinity,
    }))
    .sort((a, b) => a.r - b.r || a.i - b.i)
    .map((entry) => entry.playlist);
  if (buckets.has(target) || ordered.length) buckets.set(target, ordered);

  const out = [];
  for (const folder of folders || []) out.push(...(buckets.get(key(folder.id)) || []));
  out.push(...(buckets.get(null) || []));
  return out;
}

export function containerIds(playlists, folders, folderId) {
  const known = new Set((folders || []).map((folder) => key(folder.id)));
  const target = key(folderId);
  return (playlists || [])
    .filter((playlist) => containerOf(playlist, known) === target)
    .map((playlist) => key(playlist?.id))
    .filter(Boolean);
}
