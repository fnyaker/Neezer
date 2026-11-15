import test from "node:test";
import assert from "node:assert/strict";

import {
  containerIds,
  groupPlaylists,
  idsOf,
  insertAt,
  moveItem,
  orderContainer,
  orderFolders,
} from "../src/lib/playlistfolders.js";

const playlist = (id, extra = {}) => ({
  id,
  title: `P${id}`,
  deezer_id: null,
  folder_id: null,
  position: null,
  ...extra,
});

const folder = (id, position = 0, title = `F${id}`) => ({ id, title, position });

test("folders keep their order and each playlist lands in its own folder", () => {
  const folders = [folder("b", 0), folder("a", 1), folder("c", 2)];
  const playlists = [
    playlist("1", { folder_id: "a" }),
    playlist("2", { folder_id: "b" }),
    playlist("3", { folder_id: "a" }),
    playlist("4"),
    playlist("5", { folder_id: "c" }),
  ];
  const { groups, unfiled } = groupPlaylists(playlists, folders);
  assert.deepEqual(groups.map((g) => g.id), ["b", "a", "c"]);
  assert.deepEqual(groups.map((g) => g.playlists.map((p) => p.id)), [["2"], ["1", "3"], ["5"]]);
  assert.deepEqual(groups.map((g) => g.count), [1, 2, 1]);
  assert.deepEqual(unfiled.map((p) => p.id), ["4"]);
});

test("an empty folder is still a group, with a zero count", () => {
  const { groups } = groupPlaylists([playlist("1")], [folder("empty", 0)]);
  assert.equal(groups.length, 1);
  assert.deepEqual(groups[0].playlists, []);
  assert.equal(groups[0].count, 0);
});

test("a Deezer import is 'imported' only while it is unfiled", () => {
  const folders = [folder("jazz", 0)];
  const playlists = [
    playlist("dz1", { deezer_id: "101" }),
    playlist("dz2", { deezer_id: "102", folder_id: "jazz" }),
    playlist("local"),
  ];
  const { groups, unfiled, imported } = groupPlaylists(playlists, folders);
  assert.deepEqual(imported.map((p) => p.id), ["dz1"]);
  assert.deepEqual(unfiled.map((p) => p.id), ["local"]);
  assert.deepEqual(groups[0].playlists.map((p) => p.id), ["dz2"]);
});

test("a playlist pointing at a folder that is gone is unfiled, never imported", () => {
  const playlists = [playlist("dz", { deezer_id: "9", folder_id: "deleted" })];
  const { groups, unfiled, imported } = groupPlaylists(playlists, [folder("kept", 0)]);
  assert.equal(groups[0].count, 0);
  assert.deepEqual(unfiled.map((p) => p.id), ["dz"]);
  assert.deepEqual(imported, []);
});

test("no folder list means everything is unfiled or imported in input order", () => {
  const playlists = [playlist("1"), playlist("2", { deezer_id: "8" }), playlist("3")];
  const { groups, unfiled, imported } = groupPlaylists(playlists, null);
  assert.deepEqual(groups, []);
  assert.deepEqual(unfiled.map((p) => p.id), ["1", "3"]);
  assert.deepEqual(imported.map((p) => p.id), ["2"]);
});

test("moveItem clamps both ends, accepts out-of-range input and never mutates", () => {
  const list = ["a", "b", "c", "d"];
  assert.deepEqual(moveItem(list, 0, 3), ["b", "c", "d", "a"]);
  assert.deepEqual(moveItem(list, 3, 0), ["d", "a", "b", "c"]);
  assert.deepEqual(moveItem(list, -4, 99), ["b", "c", "d", "a"]);
  assert.deepEqual(moveItem(list, 9, -2), ["d", "a", "b", "c"]);
  assert.deepEqual(moveItem(list, 2, 2), list);
  assert.deepEqual(moveItem([], 0, 1), []);
  assert.deepEqual(list, ["a", "b", "c", "d"]);
  const same = moveItem(list, 1, 1);
  assert.notEqual(same, list);
});

test("insertAt clamps its index and keeps the source intact", () => {
  const list = ["a", "b"];
  assert.deepEqual(insertAt(list, "c", 0), ["c", "a", "b"]);
  assert.deepEqual(insertAt(list, "c", 2), ["a", "b", "c"]);
  assert.deepEqual(insertAt(list, "c", 99), ["a", "b", "c"]);
  assert.deepEqual(insertAt(list, "c", -3), ["c", "a", "b"]);
  assert.deepEqual(list, ["a", "b"]);
});

test("idsOf keeps order, drops missing ids and stringifies uuids", () => {
  assert.deepEqual(idsOf([{ id: "a" }, { id: null }, null, { id: 7 }]), ["a", "7"]);
  assert.deepEqual(idsOf(null), []);
});

test("a 500-item move keeps every row exactly once", () => {
  const list = Array.from({ length: 500 }, (_, i) => `p${i}`);
  const moved = moveItem(list, 0, 499);
  assert.equal(moved.length, 500);
  assert.equal(new Set(moved).size, 500);
  assert.equal(moved[499], "p0");
  assert.equal(moved[0], "p1");
  const up = moveItem(list, 499, 0);
  assert.equal(up[0], "p499");
  assert.equal(up.length, 500);
});

test("orderFolders follows the given ids and appends what they omit", () => {
  const folders = [folder("a", 0), folder("b", 1), folder("c", 2)];
  assert.deepEqual(orderFolders(folders, ["c", "a"]).map((f) => f.id), ["c", "a", "b"]);
  assert.deepEqual(orderFolders(folders, ["b", "c", "a"]).map((f) => f.id), ["b", "c", "a"]);
  assert.deepEqual(orderFolders(folders, []).map((f) => f.id), ["a", "b", "c"]);
  assert.deepEqual(folders.map((f) => f.id), ["a", "b", "c"]);
});

test("orderContainer rewrites one container and keeps the folders-first layout", () => {
  const folders = [folder("one", 0), folder("two", 1)];
  const playlists = [
    playlist("a", { folder_id: "one" }),
    playlist("root1"),
    playlist("b", { folder_id: "one" }),
    playlist("c", { folder_id: "two" }),
    playlist("root2"),
    playlist("gone", { folder_id: "removed" }),
  ];
  const ordered = orderContainer(playlists, folders, "one", ["b", "a"]);
  assert.deepEqual(ordered.map((p) => p.id), ["b", "a", "c", "root1", "root2", "gone"]);
  const root = orderContainer(ordered, folders, null, ["root2", "root1", "gone"]);
  assert.deepEqual(root.map((p) => p.id), ["b", "a", "c", "root2", "root1", "gone"]);
  assert.deepEqual(playlists.map((p) => p.id), ["a", "root1", "b", "c", "root2", "gone"]);
});

test("containerIds names one container in its current order", () => {
  const folders = [folder("one", 0)];
  const playlists = [
    playlist("a", { folder_id: "one" }),
    playlist("root"),
    playlist("b", { folder_id: "one" }),
  ];
  assert.deepEqual(containerIds(playlists, folders, "one"), ["a", "b"]);
  assert.deepEqual(containerIds(playlists, folders, null), ["root"]);
});
