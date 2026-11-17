// The benches' route matchers decide on the request's HOST or PATH. A name that appears in a
// query is not that host, and a host whose letters merely end with a name is not that host
// either: a bench that matched on the whole URL string could intercept (or block) a request it
// never meant (CodeQL js/regex/missing-regexp-anchor).
import test from "node:test";
import assert from "node:assert/strict";
import { deezerApi, dzcdn, deezerHost, pathOn } from "./harness.mjs";

const u = (s) => new URL(s);

test("Deezer's hosts are matched by name", () => {
  assert.ok(deezerApi(u("https://api.deezer.com/album/1")));
  assert.ok(dzcdn(u("https://e-cdns-images.dzcdn.net/images/cover/x/1000x1000.jpg")));
  assert.ok(dzcdn(u("https://dzcdn.net/x")));
  assert.ok(deezerHost(u("https://api.deezer.com/x")));
  assert.ok(deezerHost(u("https://cdns-images.dzcdn.net/x")));
});

test("a URL whose query names Deezer is not Deezer", () => {
  assert.equal(deezerHost(u("http://127.0.0.1:5000/app/?next=api.deezer.com")), false);
  assert.equal(dzcdn(u("http://127.0.0.1:5000/app/?cover=e-cdns-images.dzcdn.net")), false);
});

test("a host that merely ends with the name's letters is not Deezer", () => {
  assert.equal(dzcdn(u("https://notdzcdn.net/x")), false);
  assert.equal(dzcdn(u("https://dzcdn.net.evil.test/x")), false);
  assert.equal(deezerApi(u("https://api.deezer.com.evil.test/x")), false);
});

test("a path matcher reads the pathname only", () => {
  const stream = pathOn("/api/stream/");
  assert.ok(stream(u("http://127.0.0.1:5000/api/stream/abc")));
  assert.equal(stream(u("http://127.0.0.1:5000/app/?u=/api/stream/")), false);
  assert.equal(stream(u("http://127.0.0.1:5000/api/stream")), false);
});
