// Every URL this page fetches for its own analysis work — a track's stream, the measuring
// core, the operator's extractor — is its OWN server's. So each one is resolved against this
// page and confined to it: a value that arrived in a payload or in a Worker message can then
// never steer a request (nor the cookies it carries) to another host.

const here = () =>
  typeof location !== "undefined" && location.href ? location.href : "http://localhost/";

/**
 * `url` resolved against `base` (this page by default) and confined to it.
 *
 * Built as `origin + pathname + search` rather than the parsed `href`: a pathname starts with
 * a single slash, so the result can never name another host — not `//host`, not `user@host` —
 * and the userinfo and fragment of a crafted URL are dropped with it. That shape is also what
 * CodeQL's client-side-request-forgery query accepts as a confined URL, so keep it.
 *
 * @param {string} url
 * @param {string} [base]
 * @returns {string} an absolute same-origin URL
 */
export function ownUrl(url, base = here()) {
  const home = new URL(base);
  const resolved = new URL(url, home);
  if (resolved.origin !== home.origin) {
    throw new Error(`refused a cross-origin fetch: ${resolved.origin}`);
  }
  return resolved.origin + resolved.pathname + resolved.search;
}
