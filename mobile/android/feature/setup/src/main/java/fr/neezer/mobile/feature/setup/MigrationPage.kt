package fr.neezer.mobile.feature.setup

/**
 * The page the hidden WebView loads, served LOCALLY (lot MB4, step 4 of the
 * migration).
 *
 * The old app's SPA kept its settings in `localStorage` and its downloads in
 * IndexedDB, under the SERVER's origin. Reading them needs a page on that same
 * origin — so the WebView asks for `<origin>/__nsmigrate`, `shouldInterceptRequest`
 * answers with THIS document, and nothing ever reaches the network: the
 * interceptor refuses every other URL, and this page references no asset.
 *
 * The page reports once, through `console.log` — an EVENT, not a poll (a
 * `WebChromeClient.onConsoleMessage` handler on the Kotlin side). `getAll()` on
 * the `meta` store gives the downloads' keys and their accounts, and the albums
 * they belong to (the SPA's `slimTrack` carries `track.album.id`), which is what
 * "tes albums hors ligne" means to a person.
 *
 * Nothing here invents a value: a key the SPA stored is returned verbatim (its
 * raw JSON text), an IndexedDB it never opened answers `{}`, and a browser
 * without IndexedDB answers the same.
 */
internal object MigrationPage {
    /** The sentinel the Kotlin side waits for. */
    const val PREFIX = "NZMIGRATE:"

    val HTML: String = """
<!doctype html>
<html lang="fr">
<head><meta charset="utf-8"><title>Migration Neezer</title></head>
<body>
<script>
(function () {
  function settings() {
    var out = {};
    try {
      for (var i = 0; i < localStorage.length; i++) {
        var key = localStorage.key(i);
        if (!key) continue;
        var value = localStorage.getItem(key);
        if (value === null) continue;
        out[key] = value;
      }
    } catch (e) { /* a storage nobody may read: an empty object, never a crash */ }
    return out;
  }

  function report(payload) {
    payload.settings = settings();
    console.log("${'$'}PREFIX_PLACEHOLDER" + JSON.stringify(payload));
  }

  function readMeta(done) {
    var result = { downloads: [], albums: 0 };
    var open;
    try {
      if (!window.indexedDB) return done(result);
      open = indexedDB.open("nsupy-offline");
    } catch (e) { return done(result); }
    open.onerror = function () { done(result); };
    open.onsuccess = function () {
      var db = open.result;
      try {
        if (!db.objectStoreNames.contains("meta")) { db.close(); return done(result); }
        var request = db.transaction(["meta"], "readonly").objectStore("meta").getAll();
        request.onerror = function () { db.close(); done(result); };
        request.onsuccess = function () {
          var rows = request.result || [];
          var albums = {};
          for (var i = 0; i < rows.length && i < 4000; i++) {
            var row = rows[i] || {};
            var id = String(row.id || "");
            if (!id) continue;
            result.downloads.push({ id: id, owner: String(row.owner || "") });
            var track = row.track || {};
            var album = track.album || {};
            if (album.id) albums[String(album.id)] = true;
          }
          result.albums = Object.keys(albums).length;
          db.close();
          done(result);
        };
      } catch (e) { try { db.close(); } catch (ignored) {} done(result); }
    };
  }

  readMeta(function (data) {
    try { report(data); } catch (e) {
      try { report({ downloads: [], albums: 0 }); } catch (ignored) {}
    }
  });
})();
</script>
</body>
</html>
""".trimIndent().replace("\$PREFIX_PLACEHOLDER", PREFIX)
}
