# LAN nodes: native apps exchange their audio on the local network

Reference notes for agents working on the LAN exchange between the native apps (`desktop/` and
`mobile/`) and on its server side. Read [CLAUDE.md](../../CLAUDE.md) first: its process and seven
rules apply here. Other areas: [docs/agents/](./). The apps that become nodes are
[desktop.md](desktop.md) and [mobile.md](mobile.md); the downloads worker is `nz-offline` (desktop
*D13*, mobile *SC4*); the server's priorities and capabilities are [server.md](server.md); accounts,
groups and access are [providers.md](providers.md); the existing pairing-by-link (remote control,
party, public playlist) is [party-remote.md](party-remote.md).

**Status (2026-11-18): LN0 delivered, the rest planned.** LN0 landed every contract and every stub
(`nz-lan`, `Feature::Lan`, `Api::gates()`, the `nz-model` payloads, the Android module and routes) with
no behaviour; nothing else of this exists in code yet. This note is the plan the coding lots follow, in
order (*The lots*). Each lot, when it lands, replaces its plan paragraph by what was learned (rule, why,
number) — a delivered lot is not a changelog.

## The need

The person goes on a trip. Their PC has a lot of disk and has downloaded a great many tracks; their
phone has little room. On the same LAN (home Wi-Fi before leaving, a holiday flat, the phone's own
hotspot with the laptop on it), the two apps must **find each other securely** and **exchange
files**, with no server involved day to day:

- when a track is loaded (to play it or to download it), the app first asks the devices on the LAN
  whether they have the file; if one does, that peer serves it (faster than the server, and it works
  when the server is out of reach);
- downloading one or several **playlists** looks on the LAN first; a peer that has the playlist
  **complete** locally serves the whole of it;
- a setting **« Qualités tolérées »**: the qualities accepted from a peer **while the server is
  reachable**. For each track played or downloaded: if the peer only has a quality outside the
  tolerated list, the server is used when reachable; otherwise the peer's quality is taken;
- the cover art and the metadata come along, so the playlist looks the same offline;
- trusted devices only: **the same account's devices, and the devices of people added as trusted
  friends** (a friend sees only what they were allowed);
- secure, and **transparent day to day**: the one thing that may need the server is fetching, once
  and then silently from time to time, the key that lets the device authenticate on the LAN.

Who, where, which network: a phone on a flaky link is the default case (CLAUDE.md step 1). Here the
LAN is often **better** than the WAN (Wi-Fi at 20–80 MB/s against a 1–5 MB/s mobile link), or the
only network that works at all (a trip without data). The LAN path must therefore never make a tap
slower when it fails: it answers from memory, fails fast, and falls back to the server (rule 3).

## Research (sources) and what it decided

| Topic | Sources | Decision |
|---|---|---|
| Discovery protocol | [RFC 6762 mDNS](https://www.rfc-editor.org/rfc/rfc6762), [RFC 6763 DNS-SD](https://www.rfc-editor.org/rfc/rfc6763), [Spotify Connect ZeroConf API](https://developer.spotify.com/documentation/commercial-hardware/implementation/guides/zeroconf) (`_spotify-connect._tcp`, SRV + TXT `CPath`), [LocalSend protocol](https://github.com/localsend/protocol) (UDP multicast 224.0.0.167:53317, "some Android devices reject other groups"), [KDE Connect](https://userbase.kde.org/KDEConnect) (UDP broadcast 1716 + mDNS `_kdeconnect._udp`), [Syncthing local discovery](https://docs.syncthing.net/dev/device-ids.html) (broadcast, unauthenticated) | **DNS-SD over mDNS**, service type `_neezer-lan._udp.local.`. It is what Android (NSD), iOS (Bonjour, no special entitlement) and Linux (`mdns-sd`, Avahi-compatible) all speak natively; a home-made multicast group (LocalSend) needs the iOS multicast entitlement and is filtered by some Android builds. Discovery is **only a hint**: an announcement proves nothing (Syncthing's own security note), authentication happens on the connection. |
| Discovery privacy | [draft-ietf-dnssd-prireq](https://datatracker.ietf.org/doc/html/draft-ietf-dnssd-prireq-03) (instance name, host name and TXT leak identity), [draft-ietf-dnssd-privacy](https://datatracker.ietf.org/doc/draft-ietf-dnssd-privacy/01/), [AirDrop leak, USENIX Security 2021 (PrivateDrop)](https://tuprints.ulb.tu-darmstadt.de/20599/1/sec21-heinrich.pdf), [Matthew Green, AirDrop tracing (2024)](https://blog.cryptographyengineering.com/2024/01/11/attack-of-the-week-airdrop-tracing/) | The announcement carries **nothing identifying**: a random instance name and host name re-drawn at every network change, TXT `v=1` only — no user, no device name, no hash of an identifier (AirDrop's truncated hashes were reversed by dictionary). A stranger learns "a Neezer node is at this address", and nothing else. |
| Android discovery | [NsdManager](https://developer.android.com/reference/android/net/nsd/NsdManager), [WifiManager.MulticastLock](https://developer.android.com/reference/android/net/wifi/WifiManager.MulticastLock), [AOSP "Take the multicast lock on mDNS usage"](https://gerrit.omnirom.org/plugins/gitiles/android_packages_modules_Connectivity/+/a8efbe0e3237d0bbae738fb0ce55530977267759%5E%21) (the system takes it itself only for importance ≥ FOREGROUND_SERVICE, from T extensions 7), [chainfire on Android multicast](https://chainfire.eu/articles/742/_dev_Android_Datagrams_UDP_Broadcast_and_Multicast_.) | On Android the **platform's NSD daemon** browses and registers (Kotlin, `NsdManager`), and feeds the Rust core (`Discovery::External`). The app holds a `MulticastLock` (permission `CHANGE_WIFI_MULTICAST_STATE`) **only while browsing**, because API 26–32 and un-updated API 33 need it and the system only takes it for a foreground service. |
| iOS later | [WWDC20 "Support local network privacy"](https://developer.apple.com/videos/play/wwdc2020/10110/), [Apple: multicast networking](https://developer.apple.com/news/?id=0oi77447), [Google Cast iOS permission notes](https://developers.google.com/cast/docs/ios_sender/ios_permissions_changes) | Bonjour (`NWBrowser`) with `NSBonjourServices = [_neezer-lan._udp]` and `NSLocalNetworkUsageDescription`; **no** `com.apple.developer.networking.multicast` entitlement needed because we never send raw multicast. Same `Discovery::External` seam as Android. |
| Transport | [RFC 9000 QUIC](https://www.rfc-editor.org/rfc/rfc9000), [RFC 9001 QUIC-TLS §9.2 0-RTT replay](https://www.rfc-editor.org/rfc/rfc9001), [quinn](https://github.com/quinn-rs/quinn), [iroh TLS verifier](https://docs.rs/crate/iroh/1.0.0/source/src/tls/verifier.rs) (raw keys, TLS 1.3 only), [rustls `ClientCertVerifier`](https://docs.rs/crate/rustls/latest/source/src/verify.rs), [libp2p Noise spec](https://github.com/libp2p/specs/blob/master/noise/README.md), [Noise framework intro (Duo)](https://duo.com/labs/tech-notes/noise-protocol-framework-intro), [snow](https://docs.rs/snow) (unaudited, says so) | **QUIC (quinn 0.11.12) with TLS 1.3, 0-RTT disabled.** Multiplexed streams (a control request never waits behind a 30 MB file), per-stream flow control, loss recovery on a lossy Wi-Fi, encryption — all from one audited stack already in both `Cargo.lock`s (rustls 0.23.46 + aws-lc-rs, the provider `nz-api` already uses). Noise (snow) would need our own framing, flow control and a crate that is not audited. iroh/libp2p bring relays, hole punching and DHTs a LAN does not need (no NAT between two devices on one subnet) and a second dependency tree of several hundred crates. |
| Authentication | [Syncthing device IDs and security](https://docs.syncthing.net/dev/device-ids.html) (identity = key, mutual TLS, discovery not trusted), [KDE Connect pairing](https://userbase.kde.org/KDEConnect) (TOFU on certificate fingerprints), [LocalSend fingerprints](https://github.com/localsend/protocol) (identifier only, not authentication), [RFC 5705 / RFC 8446 §7.5 exporters](https://www.rfc-editor.org/rfc/rfc8446#section-7.5), [RFC 9266 tls-exporter channel binding](https://www.rfc-editor.org/rfc/rfc9266) | No TOFU and no pairing ceremony (the person asked for transparency, and TOFU on a hotel Wi-Fi is exactly where a MITM sits). Instead **the server is the authority**: each device holds an Ed25519 **device key** and a short **credential** signed by the server's LAN key (*Trust chain*). On the QUIC connection each side proves its device key by **signing the TLS exporter of THIS connection** — a MITM terminates two TLS sessions with two different exporters, so the signature it relays fails; a recorded proof is useless on any other connection (no replay). |
| Lifetime and revocation | [RFC 9608 noRevAvail](https://www.rfc-editor.org/rfc/rfc9608.html) (short-lived certificates make revocation unnecessary), [Syncthing security](https://manpages.ubuntu.com/manpages/noble/man7/syncthing-security.7.html) | Credentials live **30 days** (admin setting, 7–90), renewed silently every day the server is reachable. Revocation is a **signed revocation list** (serial number, monotonic) fetched with the roster and **gossiped between peers**, so a revoked device is refused by every peer that met any device which saw the server after the revocation. The residual window (a peer that met nobody since) is bounded by the credential's expiry. |
| Integrity, resume | [BLAKE3/bao verified streaming](https://docs.rs/crate/bao-tree/latest) (Merkle outboards), [tvix verified streaming note](https://git.dgnum.eu/mdebray/tvl-depot/raw/commit/06db871bd75a714a61434dd4b13bc87e1319ba92/tvix/castore/docs/verified-streaming.md), RFC 9000 (AEAD per packet) | QUIC already authenticates every byte in transit. What is left is **a peer's own file being wrong** (truncated, rewritten between a fetch and its resume): each indexed file has a **SHA-256** (aws-lc-rs, hardware-accelerated, no new crate) computed in the background; a fetch announces `(len, sha256)` and a resume carries them back, so a resume onto a different file is refused. The received file is then **probed by the decoder** (codec and duration) before it is published. A Merkle tree (bao) only pays when an untrusted *third* party relays the bytes — not our case: the peer is authenticated, and the server's on-demand Opus transcodes are not byte-reproducible, so no server-side hash could certify them anyway. |
| Reference apps | [Syncthing](https://docs.syncthing.net/dev/device-ids.html), [LocalSend](https://github.com/localsend/protocol), [KDE Connect](https://userbase.kde.org/KDEConnect), [Spotify Connect](https://developer.spotify.com/documentation/commercial-hardware/implementation/guides/zeroconf), [AirDrop research](https://tuprints.ulb.tu-darmstadt.de/20599/1/sec21-heinrich.pdf) | Taken: DNS-SD (Spotify Connect, KDE Connect), mutual authentication by key on the connection (Syncthing), "discovery is not trust" (Syncthing), either side may connect (KDE Connect). Avoided: unauthenticated HTTP fallback (LocalSend), identity hashes in announcements (AirDrop), TOFU pairing (KDE Connect, LocalGo). |
| Hotel / public Wi-Fi | [Meraki wireless client isolation](https://documentation.meraki.com/MR/Design_and_Configure/Firewall_and_Traffic_Shaping/Wireless_Client_Isolation), [Ruckus client isolation](https://support.ruckuswireless.com/articles/000014743), [Juniper Mist isolation](https://www.mist.com/documentation/isolation/), AirSnitch (breaking isolation, 2025) | Guest networks usually **isolate clients**: no unicast and no multicast between two guests. Nothing can fix that from an app; the app says it (« Aucun appareil trouvé : sur un Wi-Fi d'hôtel les appareils sont souvent isolés — partagez la connexion du téléphone ») and the phone's hotspot is the documented way. Isolation is also NOT a security boundary we rely on: the protocol is safe on a hostile open Wi-Fi. |
| Mobile constraints | [Doze and App Standby](https://developer.android.com/training/monitoring-device-state/doze-standby), MulticastLock docs (locks cost battery; released at process death) | Browse and announce **only while the app is in the foreground, or playing, or the downloads worker runs**; idle QUIC connections close after 60 s in the background; no keep-alive with the screen off; nothing schedules a wake-up for the LAN. |

**Measured on a throwaway prototype** (2026-11-18, this machine, debug build, never committed, under
`~/.cache`): `quinn =0.11.12` + `rustls 0.23.46` (TLS 1.3 only, `max_early_data_size = 0`, ALPN
`neezer-lan/1`), an ephemeral self-signed Ed25519 certificate per endpoint (`rcgen 0.14.10`),
`mdns-sd 0.17.2`, and the exporter-bound Ed25519 proof of *Transport and authentication*. Two
processes inside a **user network namespace** (`unshare -rn`, `lo` up with multicast, a route for
224.0.0.0/4 — no sudo), 5 runs:

- mDNS browse → resolved: **480–650 ms**; QUIC handshake: **2.8–4.2 ms**; handshake + both proofs
  exchanged and verified: **19–22 ms** (debug).
- A proof with one flipped bit: refused by the responder, **0 bytes** of answer.
- The credential, revocation-list and proof formats of *Trust chain* produce **byte-identical
  signatures** from Python `pycryptodome 4.4` (already a server dependency: `eddsa.new(key,
  'rfc8032')`) and from Rust (Ed25519 RFC 8032 is deterministic) — the test vectors below.

**Pitfalls found by building, not by reading:**

- `aws-lc-rs` is the rustls provider ALREADY compiled in both apps (`nz-api` depends on it; `ring` is in
  the locks but not built). Enabling rustls's `ring` feature as well makes the process-default
  provider ambiguous and `ClientConfig::builder()` panics. **nz-lan always uses
  `builder_with_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))`, quinn's
  `rustls-aws-lc-rs` feature, rcgen's `aws_lc_rs` feature, and never calls `install_default`.**
- `quinn` is in both `Cargo.lock`s but not compiled today (an optional dependency): asking for
  `=0.11.12` adds no package to either lock, only compile time.
- `mdns-sd` 0.17 hands addresses back as `ScopedIp` (not `Copy`; an IPv6 link-local address carries
  its interface scope, which a `SocketAddr` without it cannot reach).
- `RecvStream::read_exact` is async in quinn 0.11 (a 0.10 example does not compile).
- Omarchy ships **ufw with `DEFAULT_INPUT_POLICY="DROP"`** (this machine: only LocalSend's 53317
  and a few local services are open). mDNS multicast to 224.0.0.251:5353 is accepted by ufw's
  `before.rules`, so discovery works, but **an inbound QUIC connection to the desktop is dropped**.
  Hence: either side may initiate and one connection serves both directions; a fixed default port
  (`45910/udp`) the person can open with one command, which Réglages shows.

## Threat model

**Assets**: the audio files and their metadata and covers; the *list* of what a person has
(library, playlists, listening taste); the identity of the person and of their devices; the
device's key and credential; the person's bandwidth, battery and CPU (rule 1).

**Trusted**: the Neezer server the account belongs to (it is already trusted with everything); the
person's own devices while their credentials are valid and not revoked; a trusted friend, **only**
for what the person shared with them.

**Adversaries and what each must NOT achieve** (each line is a test in lot LN15):

| # | Adversary | Must not | How it is stopped |
|---|---|---|---|
| T1 | A stranger on the same Wi-Fi (passive) | learn who is there, what they have, what they play | mDNS carries a random name and `v=1` only; everything after discovery is TLS 1.3 (certificates encrypted too); the file requests are inside it. Residual: the stranger sees that a Neezer node exists, and traffic volumes. |
| T2 | A stranger who connects to a node | obtain any file, list, title, name or identifier of the node | The **initiator authenticates first**; the responder answers nothing — not even its own credential — until the initiator's proof and credential verify and the subject is authorised. Before that: one stream, ≤ 2 KiB read, 2 s deadline, then close with the same code whatever failed. |
| T3 | A stranger who answers as a node (spoofed mDNS, rogue AP) | obtain anything from the initiator; make the app play its audio | The initiator verifies the responder's proof and credential before sending any request; a responder that is not of the same account or a friend who shares with us is dropped. Residual: a rogue responder receives the initiator's credential (opaque subject id, device key, dates — no name, no content); credentials and device keys rotate at every renewal (≤ 30 days), which bounds tracking. |
| T4 | A MITM between two of my devices | read or alter the exchange, or relay a proof | Proofs sign the TLS exporter of the connection they travel on: a relayed proof does not verify on the other leg. |
| T5 | Replay of a recorded handshake or request | be accepted | Exporter-bound proofs; QUIC 0-RTT disabled (RFC 9001 §9.2); requests only exist inside an authenticated connection. |
| T6 | A stolen or lost device, revoked on the server | keep fetching from my other devices | Revocation list, signed, monotonic serial, gossiped peer to peer; credentials expire (default 30 days). Residual window documented and configurable. |
| T7 | A trusted friend's device | see or fetch anything outside what I shared with them; learn whether I have a track I did not share | Scope is enforced by the responder from the **server-signed roster**; an out-of-scope request is answered **exactly like an absent file** (same frame, same code path, from the same in-memory index). Inventory and sets sent to a friend are filtered the same way. |
| T8 | A device of another account on the same server, not a friend | anything | Its subject is neither mine nor in my roster: refused at T2's step. |
| T9 | A hostile authorised peer (bug or compromise) | crash or exhaust the node; fill the disk; make a bad file permanent | Frame size caps, stream and rate caps, timeouts, `len` checked against the free disk before writing, SHA-256 checked at the end and on resume, decoder probe before publication, the peer is marked bad for the session after one bad file. |
| T10 | A forged credential (wrong key, other server, expired, tampered) | be accepted | Ed25519 signature by a known LAN key id; validity window with ±10 min skew; magic/version/length exact. |
| T11 | Someone who copies the device's data directory | impersonate the device forever | Key file `0600` in app-private storage, excluded from Android backup; revocation from Réglages on any other device or the web UI; expiry. |

**Out of scope, stated**: a malicious server (it already holds everything); a compromised OS on the
person's own device; traffic analysis of volumes and timing on the LAN; client isolation on guest
Wi-Fi (an availability problem, not a confidentiality one).

## Architecture

```
            server (once, then from time to time; never needed to exchange)
            ├─ LAN key (Ed25519, kid)  ── signs ──►  credential(device) · roster(user) · revocations
            └─ routes /api/lan/*  (enroll, roster, devices, friends, settings)
                       ▲ nz-api (Lane::Bulk)
   ┌───────────────────┴───────────────────┐         ┌──────────────────────────────┐
   │ app (desktop GPUI  /  mobile nz-core)  │         │ another node                 │
   │  nz-player · nz-audio  ◄─ PeerFirstOpener ──┐   │                              │
   │  nz-offline           ◄─ PeerFirstDownloader│   │                              │
   │                         nz-lan ─────────────┴── QUIC/TLS1.3 + exporter proofs ─┤
   │  discovery: mdns-sd (desktop) | NSD→External (Android) | last-known addresses   │
   └────────────────────────────────────────┘         └──────────────────────────────┘
```

### One shared crate: `desktop/crates/nz-lan`

Everything that is not UI lives in **`desktop/crates/nz-lan`**, a member of the desktop workspace
(the `crates/*` glob picks it up without touching `desktop/Cargo.toml`'s member list) and a path
dependency of `mobile/rust` (`nz-lan = { path = "../../desktop/crates/nz-lan",
default-features = false }`). It compiles for Android: no GPUI, no cpal (it asks `nz-audio` with
`default-features = false`), and `mdns-sd` sits behind the default feature `mdns` that the mobile
build does not enable.

```
desktop/crates/nz-lan/
  Cargo.toml          # features: default = ["mdns"]; mdns = ["dep:mdns-sd"]
  src/
    lib.rs            # module list + re-exports (LN0)
    types.rs          # shared types: DeviceKeyPub, Subject, Kid, Urgency, Tolerance, LanError (LN0)
    proto.rs          # every wire frame, serde (LN0, final)
    cred.rs           # credential / revocation list / roster: encode, decode, verify (LN2)
    keys.rs           # the device key file: create, load, rotate, wipe (LN2)
    codec.rs          # length-prefixed frames on a QUIC stream, caps (LN6)
    transport.rs      # quinn endpoint, TLS configs, connect/accept, connection table (LN6)
    auth.rs           # the exporter-bound proof exchange, authorisation decision (LN6)
    discovery.rs      # Discovery trait, External feed, last-known addresses (LN3)
    discovery_mdns.rs # mdns-sd browse + register, behind feature "mdns" (LN3)
    store.rs          # lan.sqlite: own-file hashes, peers' last addresses, trust blobs (LN4)
    index.rs          # local inventory from nz-cache (downloads + play cache), versions (LN4)
    hashing.rs        # background SHA-256 of local files, niced, yields (LN4)
    scope.rs          # what a subject may see: own / friend scope / nothing (LN4)
    serve.rs          # responder: Inventory, Sets, Meta, Cover, Fetch; serve limits (LN4)
    peers.rs          # connected peers, replicated inventories (counted bytes) (LN9)
    decide.rs         # source decision: local / peer / server, with the tolerance (LN9)
    fetch.rs          # requester: fetch into a file, resume, hash, probe (LN9)
    enroll.rs         # enrolment, renewal, roster + revocation sync with the server (LN7)
    gossip.rs         # revocation list + own roster exchanged between peers (LN7)
    settings.rs       # kv keys and defaults (LN7)
    service.rs        # `Lan`: lifecycle, runtime, snapshot, events, shutdown joins (LN10)
    opener.rs         # PeerFirstOpener: nz_audio::StreamOpener (LN11)
    downloader.rs     # PeerFirstDownloader: nz_offline::Downloader (LN11)
    sets.rs           # playlists from a peer: list, choose, completeness (LN11)
    import.rs         # covers and metadata copied from a peer (LN11)
  tests/              # one file per lot (named in each lot)
  examples/lan_node.rs  # headless node for the two-node harness (LN15)
  tools/netns.sh        # user-namespace LAN for tests, no sudo (LN3)
```

### Trust chain

**The server's LAN key.** One Ed25519 key per server, created on first use, its 32-byte seed stored
in `Meta` under `lan_ca_seed`, encrypted with AES-GCM under `sha256(get_secret_key("lan_ca_secret"))`
(the `encrypt_password` pattern of `neezer/managers/user.py`, its own derived key). Its **key id**
`kid = SHA-256(public key)[0..8]`. A rotation (admin action, or a lost secret) creates a new key:
`GET /api/lan/status` lists the current key and the previous one for 90 days, so credentials signed
by the old key stay valid until they expire. If the server secret changes and the seed cannot be
decrypted, a new key is created and every device re-enrols silently at its next contact.

**The subject.** A person is named on the LAN by an opaque 16-byte **subject**:
`HMAC-SHA256(get_secret_key("lan_subject"), "neezer-lan/1 subject" || user_id)[0..16]` — stable,
unguessable, no column, never the name. Device names and friends' names travel only inside an
authenticated connection or from the server.

**The credential** — 154 bytes, fixed layout, big-endian, signed over a domain-separated message:

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | magic `NZLC` |
| 4 | 1 | version `1` |
| 5 | 8 | `kid` |
| 13 | 16 | `subject` |
| 29 | 12 | `device_id` (the server's row id, random) |
| 41 | 32 | `device_pub` (Ed25519) |
| 73 | 8 | `not_before` (i64 unix seconds) |
| 81 | 8 | `not_after` (i64) |
| 89 | 1 | `flags` (bit 0 may serve, bit 1 may fetch; other bits 0) |
| 90 | 64 | `sig = Ed25519(LAN key, b"neezer-lan/1 credential\x00" ‖ bytes[0..90])` |

Verification (`cred::Credential::verify`): exact length 154, magic, version 1, flags' reserved bits
0, `kid` among the known keys, signature, `not_before − 600 ≤ now ≤ not_after + 600`, `device_pub`
not in the revocation list. Every failure is ONE error variant for the peer (`Refused`), and a
precise cause for the local log and the tests.

**The revocation list** — `NZLR`, version 1, `kid`(8), `serial`(u64), `issued`(i64), `count`(u32),
then `count × 32` bytes of revoked device keys **sorted ascending, unique**, then
`sig = Ed25519(LAN key, b"neezer-lan/1 revocations\x00" ‖ body)`. Only keys whose credential has not
expired yet are listed (the list stays small: bounded by devices revoked in the last lifetime). Max
4 096 entries. A device keeps the highest serial it has seen per `kid`, from the server or from any
authenticated peer (*gossip*).

**The roster** — the per-person view the server signs, fetched by each of the person's devices:

```json
{"body": "<base64 of the canonical JSON below>", "sig": "<base64 Ed25519 over b\"neezer-lan/1 roster\\x00\" ‖ body>"}
body = {"v":1,"kid":"<hex8>","subject":"<hex16>","serial":42,"issued":1798761600,"expires":1801353600,
        "shares_to":[{"subject":"<hex16>","all":false,
                      "playlists":[{"id":"<playlist id>","title":"Route","tracks":["<track id>", …]}]}],
        "trusted_by":[{"subject":"<hex16>","name":"Alice"}]}
```

`shares_to`: the people I share with and exactly what (`all`: everything I have locally; otherwise
the union of the listed playlists' tracks, the server's snapshot at `issued`). `trusted_by`: the
people who share with me (whom my devices may ask, and their display name). The device verifies the
signature over the exact bytes received, then parses; `serial` only grows; a roster is gossiped
**only between devices of the same subject**. Caps: 64 friends, 20 000 track ids in total.

**Test vectors** (computed with both pycryptodome and ed25519-dalek, identical; LN1 and LN2 assert
them, which is what makes the Python and Rust lots compatible without waiting for each other):

```
LAN key seed   = 000102…1f (bytes 0..31)      -> pub 03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8
device seed    = 202122…3f (bytes 32..63)     -> pub 29acbae141bccaf0b22e1a94d34d0bc7361e526d0bfe12c89794bc9322966dd7
kid            = 56475aa75463474c
subject        = 00112233445566778899aabbccddeeff ; device_id = 01×12 ; not_before = 1798761600 ;
not_after      = 1801353600 ; flags = 3
credential     = 4e5a4c430156475aa75463474c00112233445566778899aabbccddeeff010101010101010101010101
                 29acbae141bccaf0b22e1a94d34d0bc7361e526d0bfe12c89794bc9322966dd7000000006b36ec80
                 000000006b5e798003a839e52b155515b9f88b70ab9844585cb0ed07a9a685618cce2243fedfce50
                 968c222fb9c69068b1ded70f7eff5f1d66acc9419e4f4b524add9fa8bf7efcde09
revocations    = (serial 7, issued 1798761600, one entry = the device pub above)
                 4e5a4c520156475aa75463474c0000000000000007000000006b36ec800000000129acbae141bccaf0
                 b22e1a94d34d0bc7361e526d0bfe12c89794bc9322966dd705c36844f72116363d35c16696464d37f6
                 88ada1a8c22f6a2f40c6a7c718da4def658ef1e6d64f8bbc5308c208fd72f661f7060efdb9dcd17a87
                 1035e0ebd005
auth proof     = Ed25519(device, b"neezer-lan/1 auth\x00" ‖ b"initiator" ‖ AA×32)
               = 8cac16e29f13ab9811527b65c630d17c0774e759eff96a937ca365f050f91b6306424a132990da39
                 4b6082123d0c7825db8adb538ee1b31242a2e036aa78b709
```

**Enrolment** (transparent): at login, and every 24 h while the server is reachable, the device
(`enroll.rs`, `Lane::Bulk`) creates its key if absent, then `POST /api/lan/enroll` with a
possession proof; the server answers the credential, the roster and the revocation list. A device
whose credential has fewer than 23 of its 30 days left renews; a renewal **rotates the device key**
(new key, new credential; the old key is listed as revoked only if the person revokes the device,
otherwise it simply expires). Logout wipes the key, the credential and the roster, and calls
`DELETE /api/lan/devices/self` when reachable.

### Discovery

- Service `_neezer-lan._udp.local.`, port = the node's QUIC port, instance name and host name
  `nz-<10 random hex>` re-drawn at every network change, TXT exactly `v=1`. Never the machine's host
  name, never a user or device name.
- **Desktop**: `mdns-sd` (feature `mdns`) browses and registers on every up, non-loopback interface.
  `MdnsDiscovery::new(true)` is **loopback only** (the tests' mode: a test must never multicast on the
  machine's own network), `new(false)` is the app's policy (LN3).
- **Android**: Kotlin `NsdManager.registerService` + `discoverServices`/`resolveService`
  (API ≥ 34: `registerServiceInfoCallback`), holding a `MulticastLock` only while browsing; each
  resolved `(host, port)` goes to `LanApi::found(host, port)`, each loss to `LanApi::lost(...)`.
- **Last-known addresses**: every authenticated peer's last `(ip, port)` is saved (`store.rs`); on a
  network change (desktop: interface list changes; Android: `ConnectivityManager` callback, already
  fed to `NzCore::set_network`) the node tries those addresses directly, once, before any mDNS
  answer — the reconnect path when multicast is filtered but unicast passes.
- **When** (battery, rule 1): discovery and announcement run while the app is in the foreground,
  while it plays, or while the downloads worker has work; they stop 60 s after the last of those.
  The desktop runs them whenever the app is open (mains-powered, cheap).
- A discovered node is dialled at once (one handshake, ~20 ms) to learn whether it is ours; a node
  that refuses us, or that we refuse, is not dialled again for 10 min (per announcement instance).
- LAN only on Wi-Fi/Ethernet/hotspot interfaces; never over the cellular interface.

### Transport and authentication

- **QUIC** (quinn 0.11.12) on UDP, dual-stack, default port **45910** (Réglages → Appareils à
  proximité → avancé, `0` = any; when 45910 is taken the node falls back to an ephemeral port and
  says so). ALPN `neezer-lan/1`. TLS 1.3 only, `max_early_data_size = 0`, `enable_early_data =
  false`. Each node creates an **ephemeral** self-signed Ed25519 certificate at start (rcgen); the
  certificate verifiers accept any certificate but still verify the TLS 1.3 handshake signature with
  it (`rustls::crypto::verify_tls13_signature`). The certificate is NOT the identity — the
  exporter-bound proof is.
- Idle timeout 30 s; keep-alive 10 s only while foreground or transferring; at most 16 connections;
  **one connection per peer pair** — when two exist (both dialled), the one dialled by the lower
  `device_pub` survives.
- **The proof exchange** (first bidirectional stream, opened by the initiator):

```
exporter_r = TLS exporter(label "EXPORTER-neezer-lan/1", context = role, 32 bytes), role ∈ {"initiator","responder"}
proof_r    = Ed25519(device key, b"neezer-lan/1 auth\x00" ‖ role ‖ exporter_r)

I → R  Auth   { cred: 154 B, proof: proof_initiator, revocation_serial: u64, proto: 1 }
R      verify cred (vs R's known kids + revocations), proof over R's own exporter("initiator"),
       authorise subject: == R's subject → Own ; in R's roster.shares_to → Friend(scope) ; else close
R → I  AuthOk { cred, proof: proof_responder, revocation_serial, proto: 1 }
I      verify the same; authorise: == I's subject → Own ; in I's roster.trusted_by → Friend ; else close
both   Hello  { name, platform, app_version, inventory_version, sets_version }   (names only now)
       then: the side with the higher revocation serial for a shared kid sends Revocations{blob};
       same subject: the side with the higher roster serial sends Roster{blob}
```

- Before `AuthOk` the responder reads at most one stream and 2 KiB, for at most 2 s; any failure
  closes the connection with application code `0x10` and an empty reason — the same for every cause.
- A friend's connection is **one-directional in content**: the responder serves only if the
  initiator's subject is in ITS `shares_to`. Two friends who share with each other each authorise
  the other on their own side.

### Protocol messages (`proto.rs`, final in LN0)

Every request opens a new bidirectional stream: one request frame, one response frame, and for
`Fetch` the raw bytes after it until FIN. A frame is `u32` big-endian length + UTF-8 JSON
(`serde_json`, `#[serde(tag = "t")]`), **≤ 1 MiB** (`Inventory`/`Sets` paginate). Unknown fields are
ignored; an unknown `t` answers `Error{code:"unknown"}` (rule 7: a newer peer talking to an older
one).

```rust
#[derive(Serialize, Deserialize)] #[serde(tag = "t")]
pub enum Request {
    Inventory { since: Option<u64>, page: u32 },          // page size 4 000 items
    Sets { page: u32 },                                    // 64 sets per page
    Meta { tracks: Vec<String> },                          // ≤ 200 ids
    Cover { url: String },                                 // ≤ 2 MiB answer
    Fetch { track: String, quality: WireQuality, from: u64, expect: Option<FileTag>, urgency: Urgency },
    Ping,
}
#[derive(Serialize, Deserialize)] #[serde(tag = "t")]
pub enum Response {
    InventoryFull { version: u64, page: u32, last: bool, items: Vec<Item> },
    InventoryDelta { from: u64, to: u64, added: Vec<Item>, removed: Vec<ItemKey> },
    Sets { version: u64, page: u32, last: bool, sets: Vec<SetInfo> },
    Meta { rows: Vec<TrackMeta> },
    Cover { mime: String, len: u32 },                      // then `len` raw bytes
    FetchOk { len: u64, tag: Option<FileTag>, quality: WireQuality }, // then raw bytes from `from`
    NotFound,                                              // absent OR out of scope: identical
    Busy { retry_in_ms: u32 },                             // serve limits (rule 1 on the responder)
    Pong,
    Error { code: String },
}
pub struct Item { pub track: String, pub copies: Vec<Copy> }
pub struct Copy { pub quality: WireQuality, pub len: u64 }
pub struct ItemKey { pub track: String, pub quality: WireQuality }
pub struct FileTag { pub len: u64, pub sha256: String }          // hex
pub struct SetInfo { pub kind: String, pub ident: String, pub title: String, pub cover: Option<String>,
                     pub tracks: Vec<String>, pub local: u32, pub complete: bool }
pub struct TrackMeta { pub id: String, pub title: String, pub artists: String, pub album: String,
                       pub album_id: Option<String>, pub duration_s: u32, pub cover: Option<String>,
                       pub explicit: bool }
#[derive(Clone, Copy)] pub enum WireQuality { Flac, Opus320, Opus128, Opus64 } // "FLAC","OPUS_320",…
#[derive(Clone, Copy)] pub enum Urgency { Play, Prefetch, Bulk }
```

- `Inventory`: everything the responder has **complete** on disk (downloads + complete play-cache
  files), per quality, filtered by the requester's scope. `since` = the requester's last version:
  a delta when the responder still has it (it keeps the last 64 changes), else a full answer.
  8 000 tracks ≈ 320 KB of JSON, sent once per peer and version.
- `Sets`: the responder's download sets (playlists, albums, shows) **whose members are in scope**,
  with their track ids and `complete` (every member has a complete local file). A friend sees only
  shared playlists (or all sets when `all`).
- `Meta` and `Cover`: only for tracks/covers in scope; `TrackMeta` is screen-shaped and sanitised
  (no likes, no play counts, no user ids).
- `Fetch`: `from` > 0 resumes; `expect` carries the `FileTag` the requester already holds — a
  different file answers `NotFound` and the requester restarts at 0 from another source.

### Authorisation and scope (`scope.rs`, the security core of the responder)

`Scope::Own` — everything. `Scope::Friend { all, tracks: HashSet<String> }` — from MY roster's
`shares_to` entry for that subject. `Scope::None` — never constructed for a live connection (the
connection was closed at authentication). Every handler takes the scope and filters; there is no
handler without it (compile-time: handlers' signature requires `&Scope`). Out of scope = `NotFound`,
built by the same function that answers an absent file.

### Choosing the source (`decide.rs`)

The decision is **local and instant**: it reads the replicated inventories already in memory, never
waits for the network to decide.

```text
input: track, wanted quality W, tolerated set T (Réglages), server_reachable (nz-api `is_online()`),
       urgency, local copies (nz-cache), peers' inventories
1. A complete local file (download, then play cache)           -> Local       (unchanged paths)
2. Candidates = peers having the track, each with its qualities
3. If a candidate has W, or a quality in T                     -> Peer(best)  best = W if present,
                                                                  else the highest quality in T;
                                                                  ties: own device before friend,
                                                                  then a peer "complete" for the
                                                                  set being downloaded, then lowest RTT
4. Else if server_reachable                                    -> Server(W)
5. Else if any candidate                                       -> Peer(highest quality it has)
6. Else                                                        -> Server(W)   (fails fast offline: rule 3)
```

`T` default: **{FLAC, Opus 320, Opus 128}** (Opus 64 is what the Auto ladder picks on a bad link;
taking it from a peer while the server could send better would be a regression). A peer that answers
`NotFound`/`Busy`/fails mid-way is skipped for this track and the decision is re-run with the rest
(one retry per peer; then the server). The opener reports which source served each track
(`LanEvent::Served`), so the UI and the tests can see it.

### Playback: peer first (`opener.rs`)

`PeerFirstOpener<O: StreamOpener>` wraps each app's opener (desktop `NetOpener`, mobile
`ServerOpener`) and implements `nz_audio::StreamOpener`:

- `local()` delegates (local files stay first).
- `open(id, quality, start, from_byte, lane)`: runs `decide`; for `Peer`, opens a `Fetch` with
  `urgency` from the lane (`User→Play`, `Prefetch→Prefetch`, `Bulk→Bulk`) and returns a
  `ByteStream` whose `len_hint()` is the file's length (a peer file is always complete, so seeks are
  byte ranges) and whose **`quality()`** (added by LN5) is the quality actually served, so the engine
  publishes the spool into the play cache under the right key. `start` (a `?start=` slice) never goes
  to a peer: a peer stream is seekable by bytes. The choice is **sticky per (track, voice)**: a seek
  re-opens from the same peer with `from_byte`.
- `ready()`: for a track the decision sends to a peer, answers `ready = true, bytes = len` from the
  inventory, without a request.
- Fails fast: no connected peer has it → straight to the inner opener, 0 ms added. A peer that
  stalls 1.5 s without a byte while the engine waits on `User` → the opener abandons it and opens
  the server (if reachable) from the byte already received, or from 0 at the peer's quality if not.

### Downloads and playlists (`downloader.rs`, `sets.rs`, `import.rs`)

- `PeerFirstDownloader` wraps `ApiDownloader` and implements `nz_offline::Downloader`; `fetch` runs
  `decide` with `urgency = Bulk`, writes the `.part` the store reserved, verifies length + SHA-256,
  probes the file, and returns `Fetched { bytes, quality }` (LN5's trait change) so the store records
  the quality actually received.
- **A playlist download** (`sets.rs`): when the person downloads a playlist, the track list comes
  from the server when reachable (today's path), otherwise from a peer's `Sets`. `Lan::best_peer_for
  (&[track])` returns the peer that has all of them (complete) — the downloader then prefers it for
  every track of that set, which is "download the playlist from the peer that has it complete".
- **« Sur vos appareils »**: a new view lists the sets the connected peers expose (own devices: all
  their download sets; friends: what they shared), each with its track count, size and « complet »;
  multi-select → « Télécharger » adds them to `nz-offline` like any download.
- **Covers and metadata** (`import.rs`): for every set taken from a peer, missing `TrackMeta` and
  covers are fetched on `Bulk` (`Meta`, `Cover`), covers stored PINNED (`Covers::put_pinned`, so the
  LRU never evicts a download's art) and titles/artists given to `nz-offline` as its `Choice` labels,
  so the downloads screen and the playlist page look the same offline.

### Rule 1 on the LAN

- **Requester**: `Urgency::Play` (the tapped track) never waits; `Prefetch` waits like the
  `Prefetch` lane; `Bulk` takes `nz-api`'s **same `Gates`** (`Api::gates()`, added by LN0): one Bulk
  transfer at a time across server AND LAN, 1 s of quiet after a `User` request, and the token bucket
  at 25 % of the playing stream's throughput while the current track still streams from the network
  (`gates.throttle(bytes)` per chunk). One mechanism, not a second throttle.
- **Responder**: serving a peer is background work for the responder. At most 2 `Play`, 1
  `Prefetch` and 1 `Bulk` transfers served at once across all peers (beyond: `Busy{retry_in_ms}`);
  `Bulk` serving waits on the responder's own `Gates` (it never slows the responder's own playback);
  reads in 64 KiB chunks on the `nz-lan` runtime thread, reniced +5 (desktop `nice`, Android
  `setpriority`), the hashing worker +10.
- The `nz-lan` runtime is its own tokio runtime (1 worker, thread `nz-lan`), owned by `Lan` and
  JOINED at shutdown; engine threads that `block_on` it are joined first (the app's shutdown order:
  engine, then offline worker, then `Lan`, then `nz-net`).

### Rules 2 and 3

- A playlist taken from a peer is an ordinary `nz-offline` job: it shows its progress, and each
  failure has its cause in French (« l'appareil "PC" ne répond plus », « fichier différent de celui
  annoncé », « qualité non tolérée et serveur injoignable » never happens — that case takes the peer).
- Enrolment, roster sync and hashing are background, silent when they succeed, and visible in
  Réglages when they fail (« Impossible de renouveler la clé LAN : serveur injoignable depuis
  3 jours »).
- Nothing on the LAN retries in a storm: a refused node waits 10 min, a failed peer is skipped for
  the track, `Busy` waits exactly `retry_in_ms`.

### Device storage (rule 7 on the device)

`$data/neezer/lan/` (desktop `$XDG_DATA_HOME`, Android `filesDir`): `device.key` (32-byte seed, mode
0600), `lan.sqlite` (WAL, `PRAGMA user_version`, idempotent migrations at start):

```sql
-- v1
CREATE TABLE IF NOT EXISTS file_tag(path TEXT PRIMARY KEY, len INTEGER NOT NULL, mtime INTEGER NOT NULL,
  sha256 TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS peer_addr(device_pub TEXT PRIMARY KEY, subject TEXT NOT NULL, name TEXT,
  ip TEXT NOT NULL, port INTEGER NOT NULL, seen_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS trust(key TEXT PRIMARY KEY, blob BLOB NOT NULL, updated_at INTEGER NOT NULL);
  -- keys: 'credential', 'roster', 'revocations:<kid>', 'ca:<kid>'
```

Settings in `nz-cache`'s `kv` (synced nowhere; per device): `lan.enabled` (default `true`),
`lan.serve` (default `true`: « Partager mes sons avec mes appareils »), `lan.tolerated` (default
`["FLAC","OPUS_320","OPUS_128"]`), `lan.port` (default `45910`). Replicated inventories live in
memory only (counted: ≤ 64 B per item).

### Server

**Schema** (one migration, SQLite + PostgreSQL + MySQL, idempotent, `SCHEMA_VERSION` = the next
date after master's at merge time — re-date on rebase if master moved):

```sql
CREATE TABLE IF NOT EXISTS lan_device (
  id CHAR(24) PRIMARY KEY,                 -- random, also the credential's device_id (12 bytes hex)
  user_id INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE,
  pubkey CHAR(64) NOT NULL UNIQUE,         -- hex Ed25519, current key (rotates on renewal)
  name VARCHAR(64) NOT NULL, platform VARCHAR(16) NOT NULL,   -- "desktop" | "android" | "ios"
  created DATETIME NOT NULL, last_seen DATETIME NOT NULL, not_after DATETIME NOT NULL,
  revoked DATETIME NULL);
CREATE INDEX IF NOT EXISTS lan_device_user ON lan_device(user_id);
CREATE TABLE IF NOT EXISTS lan_friend (
  owner_id INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE,
  friend_id INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE,
  share_all BOOLEAN NOT NULL DEFAULT FALSE, created DATETIME NOT NULL,
  PRIMARY KEY (owner_id, friend_id));
CREATE TABLE IF NOT EXISTS lan_friend_playlist (
  owner_id INTEGER NOT NULL, friend_id INTEGER NOT NULL,
  playlist_id <Playlist's id type> NOT NULL REFERENCES playlist(id) ON DELETE CASCADE,
  PRIMARY KEY (owner_id, friend_id, playlist_id));
```

(The exact column types follow `neezer/db.py`'s conventions per engine — `user` is quoted on
PostgreSQL, `DATETIME` is `TIMESTAMP` there; copy the style of the latest migration.) A revoked
device's old public keys are kept in `lan_device_key(pubkey PK, device_id FK CASCADE, not_after)`
until they expire, so the revocation list can name every key of a device whose credential may still
be valid. Meta keys: `lan_ca_seed` (encrypted), `lan_ca_prev` (previous key, encrypted, with its
end date), `lan_revocation_serial`, `lan_enabled` (default `1`), `lan_credential_days` (default 30,
7–90). The roster serial is a **column**, `user.lan_serial` (default `0`): `Meta.key` is 32
characters and `lan_roster_serial:<uuid>` is 54, which is the choice this plan left to LN1.

**Routes** (`neezer/webui/lan.py`, its own blueprint under `/api`, every route `@login_required`,
`LAN_POLICY` merged into `remote.POLICY` with every route `NEVER` — a remote-control session never
enrols, revokes or befriends; rate limits per user):

| Route | Body → answer | Rules |
|---|---|---|
| `GET /api/lan/status` | → `{enabled, subject, keys:[{kid, pub, until?}], credential_days, revocation_serial}` (`subject`: the caller's, hex) | the feature probe (`OPTIONS`); `enabled=false` hides the feature in every client |
| `POST /api/lan/enroll` | `{pubkey, name, platform, at, proof}` → `{device_id, credential, roster, revocations}` (base64) | `proof = Ed25519(device, "neezer-lan/1 enroll\x00" ‖ subject_hex ‖ pubkey_hex ‖ at)`, `|now − at| ≤ 300 s`; same device (cookie-bound `device_id` in body optional) → rotate key; ≤ 20 live devices per user (`409` + cause); 30/h per user |
| `GET /api/lan/roster` | → `{roster, revocations}` | `ETag` = `"<roster serial>-<revocation serial>"`, `304` when unchanged |
| `GET /api/lan/devices` | → `[{id, name, platform, created, last_seen, revoked, current}]` | own devices only |
| `DELETE /api/lan/devices/<id>` (`self` allowed) | → `204` | own device only (else `404`); bumps the revocation serial; audit log |
| `PATCH /api/lan/devices/<id>` | `{name}` → device | 1–64 chars |
| `GET /api/lan/friends` | → `{shares_to:[{user_id, subject, name, all, playlists:[{id,title}]}], trusted_by:[{user_id, subject, name}]}` | `user_id` is the ACCOUNT's **UUID** (String), never a number: every account id the API sends is a UUID; `subject` is the LAN's own opaque name and is a SEPARATE field |
| `POST /api/lan/friends` | `{name}` → friend | exact account name (folded); unknown or self → `404` « aucun compte de ce nom » (same answer for a disabled account); 30/h |
| `PATCH /api/lan/friends/<user_id>` | `{all?, playlists?:[id]}` → friend | `<user_id>` is the account UUID; only playlists the OWNER may read (`playlist_access`); 20 000 shared titles at most; bumps both rosters' serials |
| `DELETE /api/lan/friends/<user_id>` | → `204` | `<user_id>` is the account UUID; bumps both rosters |
| `GET/PUT /api/lan/settings` (admin) | `{enabled, credential_days}` | Meta; rule 6 |
| `POST /api/lan/rotate` (admin) | → `{kid}` | new LAN key, previous kept 90 days |

The roster serial is a per-user counter (Meta `lan_roster_serial:<user_id>`, or a column if the
lot prefers — the lot records which), bumped on any friend or scope change touching that user. A
disabled account: every device revoked (serial bump), enrol refused.

**Older servers and older apps (rule 7)**: `nz-api`'s `Feature::Lan` probes `OPTIONS
/api/lan/status`; absent (`404`) or `enabled=false` → `Lan` never starts, no screen or setting about
it is shown, and nothing else changes. An app already enrolled keeps exchanging with its peers on
the LAN with its current credential when the server is unreachable (that is the point), but stops
renewing — and stops at expiry — if the server answers `404`/`enabled=false`. Older apps (no
`nz-lan`) are unaffected: the new routes are additive, nothing existing changes shape. The protocol
carries `proto: 1`; a node refuses a different major with `0x11` and the UI says « mettez à jour
l'app sur <appareil> ».

### Screens and Réglages

French copy, glyphs through each app's icon set, no emoji, the design tokens of each app.

- **Réglages → « Appareils à proximité »** (desktop section, mobile screen): switch « Échanger avec
  mes appareils à proximité » (`lan.enabled`); switch « Partager mes sons » (`lan.serve`);
  « Qualités tolérées » (four checkboxes, at least one; explanation: « Utilisées quand un appareil
  proche a le titre et que le serveur est joignable. Sans serveur, la qualité de l'appareil est
  toujours acceptée. »); the list of devices seen now (name, own/friend, signal: « connecté »,
  latency, what is being exchanged) and of my enrolled devices (rename, « Révoquer »); « Amis de
  confiance » (add by account name, per friend: « Tout partager » or a playlist picker, remove);
  advanced: UDP port, on the desktop the firewall line (`sudo ufw allow 45910/udp`) with a copy
  button; the key's state (« Clé valable jusqu'au 18 décembre »; renewal failures with their cause).
- **« Sur vos appareils »** (desktop: a section of the downloads screen; mobile: a tab of the
  offline screen): peers' sets, multi-select, « Télécharger » (rule 2: progress in the downloads
  screen like any job).
- **Playlist page**: « Disponible complète sur PC » in the header when a peer has all of it (one
  `best_peer_for` call, in memory). No per-row badge: the shared track list stays untouched
  (rule 4's windowed lists, measured elsewhere).
- **Now playing**: the source line says « Lu depuis PC (FLAC) » when a peer serves the track.
- **The web UI (SPA)**: Réglages → « Appareils et amis »: my enrolled devices (rename, revoke), my
  trusted friends and their scope, and for admins the two server settings (rule 6). The SPA is not a
  node (a browser can neither browse mDNS nor speak raw QUIC).
- **Feedback when nothing is found** (rule 3): 10 s with no peer while I have another enrolled
  device: « Aucun appareil trouvé. Sur un Wi-Fi d'hôtel, les appareils sont souvent isolés :
  partagez la connexion du téléphone. »

## Contracts (Rust API, LN0 writes them final)

```rust
// nz-lan/src/types.rs
pub type DevicePub = [u8; 32];
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)] pub struct Subject(pub [u8; 16]);
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)] pub struct Kid(pub [u8; 8]);
pub use crate::proto::{Urgency, WireQuality};
#[derive(Clone, Debug, PartialEq, Eq)] pub struct Tolerance(pub BTreeSet<nz_api::Quality>);
impl Tolerance { pub fn default_set() -> Self; pub fn contains(&self, q: nz_api::Quality) -> bool; }
#[derive(Clone, Debug, PartialEq, Eq)] pub enum Relation { Own, Friend { name: String } }
#[derive(Clone, Debug, PartialEq, Eq)] pub enum Source { Local(PathBuf), Peer { peer: DevicePub, quality: nz_api::Quality }, Server(nz_api::Quality) }
#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum LanError {
    Disabled, Unsupported, NotEnrolled, Expired, Refused, PeerGone, NotFound, Busy { retry_in_ms: u32 },
    Mismatch, BadFile(String), Disk(String), Proto(String), Server(String), Internal(String),
}
impl LanError { pub fn cause_fr(&self) -> String; }      // every variant, a French sentence

// nz-lan/src/cred.rs
pub struct Credential { pub kid: Kid, pub subject: Subject, pub device_id: [u8; 12], pub device_pub: DevicePub,
                        pub not_before: i64, pub not_after: i64, pub flags: u8, raw: [u8; 154] }
impl Credential {
    pub fn decode(bytes: &[u8]) -> Result<Credential, CredError>;
    pub fn verify(&self, keys: &TrustKeys, revoked: &Revocations, now: i64) -> Result<(), CredError>;
    pub fn raw(&self) -> &[u8; 154];
}
pub struct TrustKeys { /* kid -> Ed25519 public key */ }
impl TrustKeys { pub fn from_status(keys: &[(Kid, [u8; 32])]) -> Self; pub fn get(&self, kid: &Kid) -> Option<&[u8; 32]>; }
pub struct Revocations { pub kid: Kid, pub serial: u64, pub issued: i64, keys: Vec<DevicePub>, raw: Vec<u8> }
impl Revocations { pub fn decode_verify(bytes: &[u8], keys: &TrustKeys) -> Result<Self, CredError>;
                   pub fn contains(&self, k: &DevicePub) -> bool; pub fn empty(kid: Kid) -> Self; }
pub struct Roster { pub kid: Kid, pub subject: Subject, pub serial: u64, pub issued: i64, pub expires: i64,
                    pub shares_to: Vec<Share>, pub trusted_by: Vec<Truster>, raw: Vec<u8> }
pub struct Share { pub subject: Subject, pub all: bool, pub playlists: Vec<SharedPlaylist> }
pub struct SharedPlaylist { pub id: String, pub title: String, pub tracks: Vec<String> }
pub struct Truster { pub subject: Subject, pub name: String }
impl Roster { pub fn decode_verify(json: &[u8], keys: &TrustKeys) -> Result<Self, CredError>;
              pub fn raw_signed(&self) -> Vec<u8>; }   // raw() = the signed body; raw_signed() = the {"body","sig"} envelope to gossip
pub fn auth_message(role: Role, exporter: &[u8; 32]) -> Vec<u8>;   // b"neezer-lan/1 auth\0" ‖ role ‖ exporter
pub enum Role { Initiator, Responder }
#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum CredError { Length, Magic, Version, Flags, UnknownKid, Signature, NotYetValid, Expired, Revoked, Json(String), Caps }

// nz-lan/src/keys.rs
pub struct DeviceKey { /* aws_lc_rs Ed25519KeyPair from a 32-byte seed */ }
impl DeviceKey {
    pub fn load_or_create(dir: &Path) -> std::io::Result<DeviceKey>;   // device.key, 0600, atomic write
    pub fn rotate(dir: &Path) -> std::io::Result<DeviceKey>;
    pub fn wipe(dir: &Path) -> std::io::Result<()>;
    pub fn public(&self) -> DevicePub;
    pub fn sign(&self, msg: &[u8]) -> [u8; 64];
    // `at` is its DECIMAL TEXT, not an i64: the two neighbours in
    // `b"neezer-lan/1 enroll\0" ‖ subject_hex ‖ pubkey_hex ‖ at` are strings, and
    // every binary field of this plan is spelled `i64`. LN1 and LN2 each assert
    // the same proof bytes (`tests/test_lan_crypto.py`,
    // `nz-lan/tests/cred.rs`).
    pub fn enroll_proof(&self, subject_hex: &str, at: i64) -> [u8; 64];
        // b"neezer-lan/1 enroll\0" ‖ subject_hex ‖ pubkey_hex (lowercase) ‖ str(at) — `at` is DECIMAL:
        // the two neighbours are strings, and this note writes `i64` where it means the binary encoding
}

// nz-lan/src/service.rs — the one handle the apps hold
pub struct LanConfig { pub data_dir: PathBuf, pub device_name: String, pub platform: &'static str,
                       pub app_version: String, pub discovery: DiscoveryKind, pub port: u16 }
pub enum DiscoveryKind { Mdns, External }                 // External: the platform feeds found/lost
#[derive(Clone)] pub struct Lan { /* Arc<Inner> */ }
impl Lan {
    pub fn start(cfg: LanConfig, api: Arc<nz_api::Api>, cache: nz_cache::Cache) -> Result<Lan, LanError>;
    pub fn set_active(&self, why: Activity, on: bool);    // Foreground | Playing | Downloading
    pub fn found(&self, host: IpAddr, port: u16);         // External discovery feed
    pub fn lost(&self, host: IpAddr, port: u16);
    pub fn network_changed(&self);
    pub fn snapshot(&self) -> LanSnapshot;                // cloned, never reads SQLite on the UI thread
    pub fn events(&self) -> tokio::sync::watch::Receiver<u64>;
    pub fn decide(&self, track: &str, wanted: nz_api::Quality, urgency: Urgency) -> Source;
    pub fn sets(&self) -> Vec<PeerSet>;                   // « Sur vos appareils »
    pub fn best_peer_for(&self, tracks: &[String]) -> Option<DevicePub>;
    pub fn settings(&self) -> LanSettings; pub fn set_settings(&self, s: LanSettings);
    pub fn enroll_now(&self);                             // after login; also runs daily by itself
    pub fn forget(&self);                                 // logout: wipe key, credential, roster
    pub fn memory(&self) -> usize;                        // counted bytes (rule: never RSS)
    pub fn shutdown(&self);                               // joins the nz-lan runtime and its threads
}
pub struct LanSnapshot { pub state: LanState, pub key_until: Option<i64>, pub key_error: Option<String>,
                         pub peers: Vec<PeerView>, pub transfers: Vec<TransferView>, pub failures: Vec<(String, String)>,
                         pub port: u16, pub port_fallback: bool }
pub enum LanState { Off, Unsupported, NotEnrolled, Searching, Connected(u32) }
pub struct PeerView { pub device_pub: DevicePub, pub name: String, pub relation: Relation, pub platform: String,
                      pub rtt_ms: u32, pub tracks: u32, pub sets: u32 }
pub struct TransferView { pub track: String, pub title: String, pub peer: String, pub direction: Direction,
                          pub bytes: u64, pub total: u64, pub urgency: Urgency }
pub struct PeerSet { pub peer: DevicePub, pub peer_name: String, pub relation: Relation, pub set: proto::SetInfo, pub bytes: u64 }
pub struct LanSettings { pub enabled: bool, pub serve: bool, pub tolerated: Tolerance, pub port: u16 }

// nz-lan/src/opener.rs, downloader.rs
pub struct PeerFirstOpener<O: nz_audio::StreamOpener> { /* inner: O, lan: Lan */ }
impl<O: nz_audio::StreamOpener> PeerFirstOpener<O> { pub fn new(inner: O, lan: Lan) -> Self; }
impl<O: nz_audio::StreamOpener> nz_audio::StreamOpener for PeerFirstOpener<O> { /* see Playback */ }
pub struct PeerFirstDownloader { /* inner: Arc<dyn Downloader>, lan: Lan */ }
impl PeerFirstDownloader { pub fn new(inner: Arc<dyn nz_offline::Downloader>, lan: Lan) -> Self; }
impl nz_offline::Downloader for PeerFirstDownloader { /* see Downloads */ }
```

**Shared-crate changes (LN0 additive, LN5 behavioural)** — both `Cargo.lock`s and both CIs:

```rust
// nz-api (LN0): Feature::Lan (probe "/lan/status", absent_fr "L'échange entre appareils proches");
//               pub fn gates(&self) -> Arc<lanes::Gates>;   endpoints/lan.rs (stubs, LN7 fills)
// nz-model (LN0): src/lan.rs — LanStatus, LanEnrollRequest/Answer, LanDevice, LanFriends, LanSettings (serde, tolerant)
// nz-audio (LN5): trait ByteStream { fn quality(&self) -> Option<Quality> { None } }
//                 the engine's StreamComplete carries `stream.quality().unwrap_or(requested)`
// nz-offline (LN5): pub struct Fetched { pub bytes: u64, pub quality: nz_api::Quality }
//                   Downloader::fetch(...) -> Result<Fetched, Outcome>   (ApiDownloader returns the asked quality)
// nz-cache (LN5): Downloads::complete_as(owner, track, bytes, quality) -> Result<PathBuf>
//                 (renames the .part to the file named for the RECEIVED quality and updates the row;
//                  `complete` keeps its signature and calls it with the reserved quality)
```

**Mobile FFI (LN13)** — `nz-ffi/src/lan.rs`, one object, one listener:

```rust
#[derive(uniffi::Object)] pub struct LanApi { /* nz_core::lan::LanCore */ }
#[uniffi::export(async_runtime = "tokio")]
impl LanApi {
    pub fn snapshot(&self) -> LanSnapshotFfi;
    pub fn subscribe(&self, l: Box<dyn LanListener>);          // pushes on change only
    pub fn found(&self, host: String, port: u16);               // NsdManager resolved
    pub fn lost(&self, host: String, port: u16);
    pub fn set_active(&self, foreground: bool, playing: bool, downloading: bool);
    pub fn settings(&self) -> LanSettingsFfi; pub fn set_settings(&self, s: LanSettingsFfi);
    pub fn peer_sets(&self) -> Vec<PeerSetFfi>;
    pub fn download_sets(&self, keys: Vec<String>) -> Result<(), NzError>;   // "<peer>:<kind>:<ident>"
    pub async fn devices(&self) -> Result<Vec<LanDeviceFfi>, NzError>;     // server, User lane
    pub async fn revoke(&self, id: String) -> Result<(), NzError>;
    pub async fn rename(&self, id: String, name: String) -> Result<(), NzError>;
    pub async fn friends(&self) -> Result<LanFriendsFfi, NzError>;
    pub async fn add_friend(&self, name: String) -> Result<(), NzError>;
    pub async fn set_friend_scope(&self, user_id: u32, all: bool, playlists: Vec<String>) -> Result<(), NzError>;
    pub async fn remove_friend(&self, user_id: u32) -> Result<(), NzError>;
    pub fn advertised_port(&self) -> u16;                       // what Kotlin registers with NSD
}
#[uniffi::export(with_foreign)] pub trait LanListener: Send + Sync { fn changed(&self, version: u64); }
// NzCore gains `pub fn lan(&self) -> Arc<LanApi>` in nz-ffi/src/lan.rs's own impl block (LN0 stub).
```

## Two nodes on one machine: the test harness

Every lot from LN6 on proves its behaviour with **two real nodes**, never with one process talking to
a mock of itself. Three rungs, cheapest first:

1. **In-process, loopback, static discovery** (`cargo test`): two `Endpoint`s (or two `Lan`s) in one
   test, each with its own temp data dir under the crate's `target/` (never `/tmp`), bound to
   `127.0.0.1:0`, told about each other with `found(127.0.0.1, port)`. This is what CI runs on every
   push; it covers authentication, scope, fetch, resume, priorities and the adapters.
2. **Two processes in a user network namespace, real mDNS** (`desktop/crates/nz-lan/tools/netns.sh`,
   LN3, delivered): no sudo — `unshare -rn` gives a network namespace with `lo` (the two processes'
   control sockets) and **one `dummy` interface** for the LAN, which is what the app announces on:

   ```sh
   unshare -rn sh -c '
     ip link set lo up
     ip link add nz0 type dummy && ip link set nz0 up
     ip addr add 10.42.0.1/24 dev nz0 && ip -6 addr add fe80::1/64 dev nz0
     ip route add 224.0.0.0/4 dev nz0 && ip -6 route add ff00::/8 dev nz0
     …'
   ```

   A **dummy interface and not `lo`**: Linux delivers an IPv4 multicast packet to a socket joined on
   `lo` but *drops the IPv6 one* (measured: `Ip6OutMcastPkts 3`, `Ip6InMcastPkts 0` for the same
   round trip that works over IPv4), so a `lo`-based harness cannot test the IPv6 link-local scope at
   all, and the app never announces on loopback anyway. The script runs the crate's own test binary
   twice (`--exact netns_announcer` / `--exact netns_browser`, role in `NZ_LAN_NETNS_ROLE`) until
   LN15's `examples/lan_node` exists: announce → a second daemon of the same process resolves it
   (`READY` is an event, never a sleep) → browse and measure → re-draw the names → goodbye.
   `--runs N` (20 by default) and `--binary PATH` (what `cargo test` hands its own test) are the whole
   interface; the logs and the FIFO live under `target/netns-ln3/`, never `/tmp`.

   Where the `dummy` module cannot be loaded from a user namespace the script falls back to `lo` and
   asks the roles for the crate's loopback mode (`NZ_LAN_NETNS_LOOPBACK=1`): IPv4 discovery is then
   still measured, and the IPv6 scope assertion prints why it is skipped (`ipv6=0`). A `veth` end is
   not a fallback: measured, its announcement carries the IPv4 address only.

   Measured on this machine, 20 runs, debug build: `browse start → Found` **p50 0 ms, p95 0 ms**;
   `start → resolvable by another daemon` **p50 856 ms, max 1 024 ms** (RFC 6762 §8.1 probing: three
   queries 250 ms apart); `stop() → the peer's Lost` **p50 1 000 ms, max 1 001 ms**. Arch allows
   unprivileged user namespaces; **GitHub's Ubuntu 24.04 runners restrict them through AppArmor**:
   the CI step first runs `sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0` (the runner
   has sudo; a developer machine never needs it). The harness **skips with a printed reason** when
   `unshare -rn true` fails locally, and **fails** in CI (`NZ_LAN_REQUIRE_NETNS=1`).
3. **Desktop ↔ Android emulator** (LN15): the emulator sits behind its own NAT (`10.0.2.x`), no
   multicast crosses it. `mobile/tools/emu.sh start --port <p>` (headless, `-no-window -no-audio`),
   then the emulator console `redir add udp:<hostport>:45910` maps a host UDP port to the guest; the
   desktop node is given `found(127.0.0.1, hostport)` (the last-known-address path), and the phone
   reaches the host at `10.0.2.2`. NSD itself is exercised on the emulator by the app and a second
   node pushed into the same emulator (`cargo ndk` build of `examples/lan_node`, `adb -s
   emulator-<p> push … /data/local/tmp/`); if the emulator's loopback does not carry multicast
   (to be measured by LN14), the NSD check runs on a real phone over `adb` and the PR says so.

Never on the user's display or network: the netns has no route out, the emulator is headless, the
ports are chosen per lot (`45910 + 10 × lot number`), and only the lot's own PIDs are killed.

## Measures (each lot that touches what they measure writes them in its PR and next to the assertions)

| Measure | How | Target |
|---|---|---|
| Discovery latency | netns harness, 20 runs, browse start → `PeerView` connected | p50 ≤ 1.0 s, p95 ≤ 2.0 s (desktop↔desktop); last-known address after a network change ≤ 300 ms |
| Handshake + proofs | `Endpoint::connect` → authenticated, loopback, release, 100 runs | p95 ≤ 15 ms loopback; ≤ 150 ms on real Wi-Fi (reported) |
| Tap → sound from a peer | `output_stats` `tap_ns` → `first_audible_ns` (mobile), engine events (desktop) | ≤ 300 ms on loopback; the decision itself adds **0 requests** and ≤ 1 ms (in-memory) |
| No peer has it | same, with peers connected but no copy | **+0 ms** against the server-only path (no LAN request is made) |
| LAN throughput | 200 MB file, release, loopback; real Wi-Fi reported | ≥ 300 MB/s loopback; ≥ 70 % of `iperf3 -u` on the same Wi-Fi |
| Control bytes | counted per action | a tap: 0 control bytes (inventory already replicated); first contact with an 8 000-track peer ≤ 400 KB once; a delta ≤ 2 KB |
| Memory | `Lan::memory()` counted bytes | ≤ 64 B per replicated item; 4 peers × 8 000 tracks ≤ 2.5 MB; flat (≤ 1 % growth) over 1 h of exchange |
| Rule 1 | play a network track while a peer pulls a 2 GB playlist from this node, and while this node pulls one | **0 underruns**, 0 rebuffers; the tapped track's first byte delay unchanged (± 10 %) vs no transfer |
| Battery (Android) | `/proc/<pid>/stat` utime+stime, 30 min, screen on idle with LAN on vs off; 30 min screen off not playing | ≤ +5 % CPU time on; screen off not playing: **0** LAN packets (connections closed, NSD stopped — `tcpdump` not needed: `Lan::snapshot` counters) |
| Security | LN15's mutant list | every mutant refused, **0 bytes** of content or identity leaked to the refused party |

## Worker rules (mandatory in every lot; each prompt repeats them in French)

1. **Git, non-interactive only**: `export GIT_EDITOR=true GIT_SEQUENCE_EDITOR=true EDITOR=true
   VISUAL=true`; `git commit -m`, `git merge --no-edit`, never `rebase -i`. **Never `git stash`**
   (the stack is shared by every worktree): set work aside with a WIP commit.
2. **Before merging**: `git fetch origin && git rebase origin/master`, then `git diff --stat
   origin/master...HEAD` must list ONLY the lot's exclusive files (plus its own section of this
   note). Anything else means a stale base or a stray edit: fix it before merging.
3. **One writer per file.** A lot writes only its exclusive files. A lot that needs another file
   changed stops and asks (`ao report --needs-input`), it does not edit it.
4. **A change to a shared type or to a shared crate's manifest** (`desktop/crates/*`) updates BOTH
   `desktop/Cargo.lock` and `mobile/rust/Cargo.lock` (`cargo update -p <crate> --offline` or a
   `cargo check --locked` that fails tells you which), and is merged only with BOTH the `desktop`
   and the `mobile` workflows green.
5. **Memory is measured in counted bytes** (`Lan::memory()`), never by the RSS of a shared process.
6. **A thread that borrows a tokio runtime is joined before that runtime stops** (the engine's and
   the offline worker's threads before `Lan`'s runtime, `Lan`'s before `nz-net`).
7. **Tests wait on events, never on the clock**: a channel, a `watch`, a state change; tokio's paused
   clock (`start_paused`) for timeouts. No `sleep` to "let it settle".
8. **Nothing on the user's display, speakers or network**: Xvfb for any desktop window, the emulator
   through `mobile/tools/emu.sh` (`-no-window -no-audio`), `adb -s emulator-<port>` always, the LAN
   tests on loopback or in `unshare -rn`, ports reserved per worker, kill only your own PIDs.
9. **Nothing big in `/tmp`** (tmpfs = RAM): builds, data dirs, captures and logs under the
   worktree's `target/`, `~/.cache` or `~/.local/share`. `CARGO_BUILD_JOBS=4`, one cargo at a time.
10. **Security first**: a refusal reveals nothing (same code, same timing class), every size and
    count from the wire is capped before allocation, nothing from the wire becomes a path, no key,
    credential, roster or cookie is ever logged.
11. **French UI copy, no emoji, the design tokens, rule 1 for every byte** (lanes / urgencies), and
    the numbers written in the PR and next to the assertions.
12. **Done means**: acceptance met and measured, CI fully green (every job of every workflow that
    ran, the `security.yaml` logs read and every new finding fixed), this note's lot section replaced
    by what was learned, PR merged **without waiting**; then `ao report --done`.

## The lots

### Order, waves and the graph

A **wave** is a set of lots whose exclusive files are disjoint and whose dependencies are merged:
they run in parallel. This machine holds **at most four Rust-building lots at once** and two
emulators. **Effort**: `max` for cryptography, authentication, authorisation and transport; `high`
otherwise (the worker model's variants: `deepseek-v4.1-flash` = high, `deepseek-v4.1-flash-max`).

| Wave | Lots (parallel) | Waits for |
|---|---|---|
| 0 | **LN0** contracts and stubs (high) | — |
| 1 | **LN1** server: schema, LAN key, routes (max) · **LN2** trust codec + device key (max) · LN3 discovery + netns harness (high) · **LN4** index, hashing, scope, serve (max) · LN5 shared-crate adaptations (high) | LN0 |
| 2 | **LN6** transport + authentication (max) · LN7 enrolment + trust sync (max) · LN8 web UI « Appareils et amis » (high) | LN2 (LN6, LN7), LN1 (LN7, LN8) |
| 3 | **LN9** requester core: peers, decision, fetch (high) | LN4, LN6 |
| 4 | **LN10** the `Lan` service (max) | LN3, LN6, LN7, LN9 |
| 5 | **LN11** adapters: opener, downloader, sets, covers/metadata (high) | LN5, LN10 |
| 6 | **LN12** desktop wiring + screens (high) · LN13 mobile core + FFI (high) | LN11 (+ mobile MB3b, MB15 for LN13) |
| 7 | LN14 Android: NSD + screens (high) | LN13, mobile MB16 |
| 8 | **LN15** security audit, mutants, the two-node journey measured (max) | every LN lot |

```mermaid
graph LR
  LN0 --> LN1 & LN2 & LN3 & LN4 & LN5
  LN2 --> LN6 & LN7
  LN1 --> LN7 & LN8
  LN4 --> LN9
  LN6 --> LN9
  LN3 --> LN10
  LN6 --> LN10
  LN7 --> LN10
  LN9 --> LN10
  LN5 --> LN11
  LN10 --> LN11
  LN11 --> LN12 & LN13
  MB3b[mobile MB3b] --> LN13
  MB15[mobile MB15] --> LN13
  LN13 --> LN14
  MB16[mobile MB16] --> LN14
  LN8 & LN12 & LN14 --> LN15
```

Critical path: LN0 → LN2 → LN6 → LN9 → LN10 → LN11 → LN12/LN13 → LN14 → LN15. LN1, LN3, LN4,
LN5, LN7 and LN8 must be merged before the wave that waits for them, so start them at once in
their wave.

### File ownership

| Lot | Exclusive files |
|---|---|
| LN0 | `desktop/crates/nz-lan/**` (all stubs: `Cargo.toml`, `src/*.rs`), `desktop/Cargo.toml` (`[workspace.dependencies]` pins only), `desktop/Cargo.lock`, `desktop/crates/nz-api/src/{features.rs,client.rs (only `gates()`)}`, `desktop/crates/nz-api/src/endpoints/{mod.rs (one line),lan.rs}`, `desktop/crates/nz-model/src/{lib.rs (one line),lan.rs}`, `mobile/rust/Cargo.toml`, `mobile/rust/Cargo.lock`, `mobile/rust/crates/{nz-core,nz-ffi}/Cargo.toml`, `mobile/rust/crates/nz-core/src/{lib.rs (one line),lan/mod.rs}`, `mobile/rust/crates/nz-ffi/src/{lib.rs (one line),lan.rs}`, `mobile/android/settings.gradle.kts` (one include), `mobile/android/app/build.gradle.kts` (one dependency), `mobile/android/app/src/main/AndroidManifest.xml` (one permission), `mobile/android/app/src/main/java/fr/neezer/mobile/app/NavGraph.kt` (two routes), `mobile/android/feature/lan/**` (stub module), `CLAUDE.md` + `AGENTS.md` (only if a count changes) |
| LN1 | `desktop/crates/nz-api/tests/bench.rs` (delete `PENDING_SERVER_ROUTES`' `Feature::Lan`, see *Acceptance*), `neezer/lan.py` (new: LAN key, subject, credential/roster/revocation encoding), `neezer/webui/lan.py` (new: routes, `LAN_POLICY`), `neezer/db.py` (models + `SCHEMA_VERSION`), `neezer/schema/{sqlite,postgres,mysql}.sql`, `neezer/schema/migration/{sqlite,postgres,mysql}/<date>.sql`, `neezer/web.py` (one registration), `tests/test_lan_*.py`, `docs/api` entry if the routes are listed there |
| LN2 | `desktop/crates/nz-lan/src/{cred.rs,keys.rs}`, `desktop/crates/nz-lan/tests/cred.rs` |
| LN3 | `desktop/crates/nz-lan/src/{discovery.rs,discovery_mdns.rs}`, `desktop/crates/nz-lan/tools/netns.sh`, `desktop/crates/nz-lan/tests/discovery.rs`, `.github/workflows/desktop.yaml` (one step: the AppArmor sysctl + the netns test) |
| LN4 | `desktop/crates/nz-lan/src/{store.rs,index.rs,hashing.rs,scope.rs,serve.rs}`, `desktop/crates/nz-lan/tests/serve.rs` |
| LN5 | `desktop/crates/nz-audio/src/{source.rs,engine.rs,events.rs}` (the `quality()` hook only), `desktop/crates/nz-offline/src/{download.rs,service.rs,testing.rs,lib.rs}`, `desktop/crates/nz-offline/tests/resume.rs`, `desktop/crates/nz-cache/src/downloads.rs`, the call sites that match on `StreamComplete` or call `Downloader::fetch` (`desktop/app/src/state.rs`, `desktop/app/src/wiring/{engine,offline}.rs`, `mobile/rust/crates/nz-core/src/player/mod.rs`, `mobile/rust/crates/nz-core/src/offline/**`) — signature adaptation only, no behaviour |
| LN6 | `desktop/crates/nz-lan/src/{codec.rs,transport.rs,auth.rs}`, `desktop/crates/nz-lan/tests/transport.rs` |
| LN7 | `desktop/crates/nz-lan/src/{enroll.rs,gossip.rs,settings.rs}`, `desktop/crates/nz-api/src/endpoints/lan.rs` (bodies), `desktop/crates/nz-lan/tests/enroll.rs` |
| LN8 | `webapp/src/components/LanSettings.svelte` (new), `webapp/src/lib/lan.js` (new), `webapp/src/routes/Settings.svelte` (one section include + its nav entry), `webapp/test/lan.test.js` (new), screenshots under `docs/agents/screenshots/lan/LN8/` |
| LN9 | `desktop/crates/nz-lan/src/{peers.rs,decide.rs,fetch.rs}`, `desktop/crates/nz-lan/tests/fetch.rs` |
| LN10 | `desktop/crates/nz-lan/src/service.rs`, `desktop/crates/nz-lan/tests/service.rs` |
| LN11 | `desktop/crates/nz-lan/src/{opener.rs,downloader.rs,sets.rs,import.rs}`, `desktop/crates/nz-lan/tests/integration.rs` |
| LN12 | `desktop/app/src/wiring/lan.rs` (new), `desktop/app/src/wiring/{mod.rs (one line),engine.rs (wrap the opener),offline.rs (wrap the downloader)}`, `desktop/app/src/screens/settings/lan.rs` (new), `desktop/app/src/screens/settings/mod.rs` (the section), `desktop/app/src/screens/peer_sets.rs` (new), `desktop/app/src/screens/mod.rs` (one line), the downloads screen's entry point to « Sur vos appareils » and the now-playing source line (one call each, in the files that own them), `docs/agents/screenshots/lan/LN12/` |
| LN13 | `mobile/rust/crates/nz-core/src/lan/**`, `mobile/rust/crates/nz-ffi/src/lan.rs`, the opener construction in `mobile/rust/crates/nz-core/src/player/mod.rs` and the downloader construction in `mobile/rust/crates/nz-core/src/offline/**` (≤ 20 lines each) |
| LN14 | `mobile/android/feature/lan/**`, one row in `mobile/android/feature/settings/**` (the entry), one tab entry in `mobile/android/feature/offline/**` (« Sur vos appareils »), `docs/agents/screenshots/lan/LN14/` |
| LN15 | `desktop/crates/nz-lan/examples/lan_node.rs`, `desktop/crates/nz-lan/tools/two_nodes.sh`, `desktop/crates/nz-lan/tests/{security.rs,journey.rs}`, this note (the measured numbers, the final threat-model table) |

Every lot also rewrites ITS OWN section below (and only that section) when it lands.

### The prompts

Each lot ends with a complete prompt for its worker. Copy a prompt whole, never trimmed: it carries
the rules block.

### LN0 — Contracts and stubs (delivered 2026-11-18)

**What landed.** Every declaration a later lot compiles against, in its FINAL shape, with no
behaviour: a body answers `Err(LanError::Unsupported)`, an empty collection or `None`, and nothing in
either app calls the crate yet (the desktop opens the same screens, the phone the same tabs).

- `desktop/crates/nz-lan`: the crate, its manifest, and one module per line of *Architecture* — 26
  Rust files (`lib.rs` + 25 modules), `proto.rs` complete (the six `Request`s, the ten `Response`s,
  the `Item`/`Copy`/`ItemKey`/
  `FileTag`/`SetInfo`/`TrackMeta`/`Hello` records, `Control`, `WireQuality`, `Urgency`), the
  transport surface LN6 fills (`Identity`, `TrustState`, `Endpoint`, `PeerConn`, `Reply`, `Body`,
  `Incoming`), and the signatures of `cred`, `keys`, `store`, `index`, `hashing`, `scope`, `serve`,
  `peers`, `decide`, `fetch`, `enroll`, `gossip`, `settings`, `service`, `opener`, `downloader`,
  `sets`, `import` exactly as their lots name them.
- `nz-api`: `Feature::Lan` (probe `/lan/status`, absent « L'échange entre appareils proches »),
  `Api::gates()` (the ONE throttle the LAN shares with the server), and `endpoints/lan.rs`'s twelve
  methods answering the `404` an absent route answers — the very failure `has_route` reads as "no
  such feature".
- `nz-model`: `src/lan.rs`, the eleven payloads of LN1's routes, every field `serde(default)`, and
  `Quality`'s `Ord` (see the first correction below).
- Mobile: `nz-lan` as a path dependency with `default-features = false` (workspace + `nz-core` +
  `nz-ffi`), the empty `nz-core::lan`, `LanApi`/`LanListener`/the quota of screen-shaped records and
  `NzCore::lan()` in `nz-ffi/src/lan.rs`, the empty `:feature:lan` Gradle module with its two entry
  composables, the two routes (`Routes.Lan`, `Routes.PeerSets`), and
  `CHANGE_WIFI_MULTICAST_STATE`.

**What a later lot can rely on.** It fills BODIES in its own files. It never edits a `Cargo.toml`, a
`lib.rs`, `proto.rs`, `types.rs`, the manifest, `NavGraph.kt` or a Gradle file; if a contract turns out
wrong, it stops and asks instead of forking a parallel type. `mdns` is the crate's only feature and
gates `mdns-sd` alone, so `--no-default-features` is what the phone builds.

**Where the contract lives now.** In the code, and the code is what a later lot reads after this
note: `nz-lan/src/proto.rs` (every frame), `types.rs` (identity, tolerance, source, error),
`cred.rs`/`keys.rs` (the trust chain), `transport.rs` (the surface LN6 fills),
`service.rs` (`Lan`, its snapshot and its settings), `nz-api/src/endpoints/lan.rs` (the twelve methods
and the lane of each), `nz-model/src/lan.rs` (the payloads) and `nz-ffi/src/lan.rs` (the FFI). The
signatures below this note's *Contracts* section are therefore a map, not a second copy: a divergence
would be the bug.

**The numbers, each with its command** (`desktop/`, `CARGO_BUILD_JOBS=4`, 2026-11-18):

| Measure | Value | Command |
|---|---|---|
| crates added to the desktop lock | **19 entries** = `nz-lan` + `rcgen` + `mdns-sd` and their dependencies; `quinn`/`quinn-proto`/`quinn-udp` appear **0** times as additions | `git diff desktop/Cargo.lock \| grep -cE '^\+name = '` |
| crates added to the mobile lock | **15 entries** = `nz-lan` + `rcgen` and its dependencies; **no** `mdns-sd`, `flume`, `if-addrs`, `socket-pktinfo` | same, on `mobile/rust/Cargo.lock` |
| distinct rustls versions | **1** (`0.23.46`, on 8 tree lines) | `cargo tree -p nz-lan -e normal --prefix none \| awk '$1=="rustls"{print $2}' \| sort -u` |
| TLS providers | **1** (`aws-lc-rs 1.19.0`); `ring` is in the lock, never in the graph | `cargo tree -p nz-lan -e normal \| grep -oE '(ring\|aws-lc-rs) v[0-9.]+' \| sort -u` |
| forbidden crates in the Android shape | **0 lines** (`gpui`, `cpal`, `zbus`, `whisper`, `mdns`) | `cargo tree -p nz-lan --no-default-features -e normal \| grep -cE 'gpui\|cpal\|zbus\|whisper\|mdns'` |
| the crate's own tests | **41 passed** | `cargo test -p nz-lan` |
| `nz-model`'s tests | **37 passed** (33 unit + 4 fixtures) | `cargo test -p nz-model` |
| `nz-api`'s bench | **6 passed**, and its log shows `OPTIONS /api/lan/status → 404`, `/api/genre/status → 200`, unknown route `→ 404` | `cargo test -p nz-api --test bench` |
| a feature probe's cost | **1 request**, **0-byte body** for a registered route (~700 B of headers), **207 bytes** of JSON for the 404 | `curl -s -o /dev/null -w '%{size_download}' -X OPTIONS http://127.0.0.1:<port>/api/lan/status` on `tools/perf_api.py --serve` |
| clippy | green, `-D warnings`, on `nz-lan` (desktop) and `--workspace --all-targets --locked` on `mobile/rust` | the CI command of each workflow |

The `cargo ndk -t arm64-v8a -P 26 check -p nz-lan --no-default-features` step of the plan was NOT run
on the author's machine (no Android SDK/NDK, no Gradle): the Android compile of `nz-lan` is proven by
`mobile.yaml`'s APK job instead, which builds `nz-ffi` → `nz-core` → `nz-lan` for **three** ABIs
(`mobile/rust/build-android.sh`).

**Rules a measurement taught.**

- **The plan's rustls check counted lines, not versions.** `grep -c 'rustls v'` on `cargo tree`'s
  output answers **11**, because `cargo tree` repeats a shared dependency once per path AND because
  `hyper-rustls`/`tokio-rustls` match the same pattern. The fact the note is really after is "one
  rustls VERSION", and it needs `--prefix none` + an exact `$1 == "rustls"` (1 line: `0.23.46`).
  A count of matching lines is not a count of versions.
- **`nz-audio` has its OWN `Quality` and `Lane`**, not `nz_api`'s. `StreamOpener`'s methods take
  `nz_audio::Quality` (four rungs) and `nz_audio::Lane`, while `nz_api::Quality` is the server's six.
  `PeerFirstOpener` therefore converts at its boundary and never swaps one enum for the other; that
  conversion is LN11's, and LN5's `ByteStream::quality()` is where it surfaces.
- **A listener is `#[uniffi::export(callback_interface)]`**, not `export(with_foreign)`: on uniffi
  0.32.2 the latter gives `Box<dyn LanListener>` no `TypeId` (**2 × E0277**), and the crate's three
  other listeners are declared the first way. One spelling for one pattern.
- **A stub must be unconstructible-or-honest.** `Lan::start` answers `Unsupported`, so no `Lan` exists
  before LN10 and `PeerFirstOpener`/`PeerFirstDownloader` cannot be built: their tests prove the SHAPE
  (`one StreamOpener`/`Downloader`, `Send + Sync`) and say so, rather than inventing a fake node.
  Likewise `Store::open` and `DeviceKey::load_or_create` refuse and write **nothing** (asserted: the
  temp directory stays empty), so a lot that forgot to fill one cannot leave a half-made database or an
  all-zero key on somebody's phone.
- **`serde(default)` is on every field of a wire frame and of a payload**, so a newer peer and a newer
  server each parse on the other side (rule 7); `proto::Urgency` travels lowercase, `WireQuality` with
  the server's own `FLAC`/`OPUS_320` spelling.

**Two corrections to this plan, accepted before the lot merged** (the note is the contract, so it is
corrected here with the number that proved it):

1. **`Tolerance(pub BTreeSet<nz_api::Quality>)` did not compile**: `nz_model::Quality` had no `Ord`
   (its derive was `Debug, Clone, Copy, Default, PartialEq, Eq, Hash`). `PartialOrd, Ord` were added
   **and the declaration order was made the quality order** (`Opus64 < Opus128 < Opus192 < Opus256 <
   Opus320 < Flac`), so `Tolerance::best_of` — "the best tolerated copy a peer has" — is
   `Quality`'s own `max()` and not a second ranking table that could disagree with it.
   `nz-model`'s test `the_declaration_order_is_the_quality_order` holds the two together, and
   `Tolerance::best_of` is the method `decide.rs` ranks candidates with.
2. **`Feature::Lan` would have turned the desktop CI red until LN1**: `nz-api`'s bench requires
   `Some(true)` for every `Feature::ALL` against the real server, and `/lan/status` does not exist
   before LN1. The bench now names the one exemption in `PENDING_SERVER_ROUTES = &[Feature::Lan]`
   (asserting a definite `Some(false)` for it, not a weaker "any answer"), keeps the explicit pair
   `/genre/status` = `Some(true)` / an unknown route = `Some(false)`, and **LN1 removes `Lan` from
   that list** — see LN1's acceptance, which carries the step.

### LN1 — Server: schema, LAN key, routes (delivered 2026-11-18)

**What landed.** The server is the LAN's authority, and it holds the only signing key.

- **`neezer/lan.py`** — the whole trust chain, with no Flask in it: the LAN key (Ed25519,
  created on first use, its 32-byte seed **sealed** in `Meta lan_ca_seed` under
  `sha256(get_secret_key("lan_ca_secret"))`, a rotation keeping the previous key 90 days in
  `lan_ca_prev`), the **subject**, `encode_credential` / `encode_revocations` / `encode_roster`,
  the enrolment proof, the device and friendship operations. `Refused` carries the French cause
  and the machine code a route answers; a caller that is refused learns nothing else. The 20 000
  track ids are capped **in total** at write time (`_shared_track_total`, one statement counted per
  friend-playlist pair), not per friend: 64 friends each sharing 4 000 titles would otherwise make
  every device launch read a quarter of a million rows.
- **`neezer/webui/lan.py`** — the twelve routes of the *Server* table, `LAN_POLICY` (every route
  `NEVER`) merged into `remote.POLICY` from the blueprint's `register`, the two budgets (30
  enrolments and 30 befriending attempts per account per hour), an 8 KiB body cap and one JSON
  error handler. `neezer/web.py` gains one registration.
- **Schema `20261124`** — `lan_device`, `lan_device_key`, `lan_friend`, `lan_friend_playlist`, and
  `user.lan_serial`. The roster counter is a **column**, not `Meta lan_roster_serial:<user_id>`:
  `Meta.key` is 32 characters and that key is 54 — the plan left the choice to this lot. The
  migration is a `.py` on all three engines (a guarded `ALTER` plus `CREATE TABLE IF NOT EXISTS`),
  so it is idempotent and the boot repair can replay it.
- **One contract correction, accepted before merge**: `nz-model`'s `LanFriend.user_id` (and the
  `nz-api`/`nz-ffi` friend calls) were `u32`, but every account id this API sends is a UUID
  (`AdminUser.id`, `ids::UserId`). They are `UserId`/`String` now, and the LAN's opaque `subject`
  travels as its own field — the trust chain never names an account by its id. `nz-api`'s bench
  also deleted its `PENDING_SERVER_ROUTES` exemption, so `Feature::ALL` is required again.

**What a later lot can rely on.** These are the parts LN7/LN8/LN13 consume: `GET /api/lan/status`
answers `{enabled, subject, keys:[{kid, pub, until?}], credential_days, revocation_serial}` and
`OPTIONS` on it is a 200, so the feature probe works before a login; `POST /api/lan/enroll` takes
`{pubkey, name, platform, at, proof, device_id?}` and answers `{device_id, credential, roster,
revocations}`, with `roster` as `{body, sig}` and the two blobs base64; `GET /api/lan/roster`
answers the same pair plus `etag` and sets the note's `"<roster>-<revocation>"` ETag (`304`, 0 body
bytes, on a match); `GET /api/lan/devices` is a bare array and `PATCH` a bare object; the friends
routes answer `{shares_to, trusted_by}` and one friend object, with `user_id` the account's UUID and
`subject` the hex; `GET|PUT /api/lan/settings` is admin-only; `POST /api/lan/rotate` answers
`{kid}`. The enrol proof signs `b"neezer-lan/1 enroll\0" ‖ subject_hex ‖ pubkey_hex ‖ at` with `at`
in **decimal text** — the same message LN2 pins, and the vector below is asserted on both sides.
A refusal is always `{"error": <French>, "code": <machine>}`, and the same body for every cause of
one kind (an unknown account, yourself and a disabled account are one 404).

**The numbers, each with its command** (SQLite, this machine, 2026-11-18; `~/.cache/neezer-ln1/venv`).

| Measure | Value | Command |
|---|---|---|
| The note's three vectors | **3 / 3 byte-identical** (credential 154 B, revocations 129 B, auth proof 64 B) | `python -m unittest tests.test_lan_crypto.VectorTestCase` |
| The enrolment vector LN2 pins | **byte-identical** (`a5f56aec…cb08`); `at` as 8-byte big-endian gives a DIFFERENT proof and is refused by the test | same file, `test_the_enrollment_proof_is_the_message_ln2_pins` |
| LAN tests | **53 passed** (20 crypto + 29 routes + 4 upgrade) | `python -m unittest tests.test_lan_crypto tests.test_lan_routes tests.test_lan_upgrade` |
| Whole Python suite | **2 548 passed, 39 skipped, 0 failed** (16 workers, 23.2 s) | `python tools/partest.py` |
| A share of ONE 4 000-title playlist | body **156 305 B** (the note's 200 KB), base64 envelope **208 408 B**, HTTP payload **208 701 B**, gzip **104 698 B** | `tests/test_lan_crypto.RosterTestCase.test_a_four_thousand_track_share_stays_under_200_kb_and_two_statements` |
| The same roster's cost | **5 SELECT**, of which **2** are the share (playlists + tracks); a 40-title share costs the **same 5** — no statement per title | same test (`count_statements`) |
| Roster build | **14.4 ms** for the 4 000-title share, signature included | `~/.cache/neezer-ln1/measure.py` |
| One enrolment | **18 statements** (14 SELECT, 2 INSERT, 1 SAVEPOINT + 1 RELEASE), credential 208 base64 chars | `~/.cache/neezer-ln1/measure2.py` |
| Migration, 1 000 existing accounts | **86.2 ms** for the boot that upgrades (schema check included); the migration alone replays in **0.46–0.75 ms** | `~/.cache/neezer-ln1/measure3.py`, 5 runs |
| `nz-model` | **37 passed** (33 unit + 4 fixtures) | `cargo test -p nz-model --locked` |
| `nz-api` + `nz-model` lints | **0 warning**, all targets | `cargo clippy -p nz-model -p nz-api --all-targets --locked -- -D warnings` |
| Formatting | clean | `cargo fmt --check` |

**Rules a measurement taught.**

- **pycryptodome's `eddsa.sign(SHA512.new(msg))` is Ed25519ph, not Ed25519.** The hash-object form
  prepends `dom2(1, "")` (RFC 8032 §5.1.7) and produces a signature no `ed25519-dalek` or
  `aws-lc-rs` verifier accepts. Passing the message as **`bytes`** is the pure form. The first
  vector did not reproduce until this was found — a mismatch that looked like a wrong note vector
  and was a wrong API call.
- **`Meta.key` is 32 characters**, so a per-account key (`lan_roster_serial:<uuid>`, 54) does not
  fit; `user.lan_serial` is the column, and `_repair_skipped_migrations` sees it like any other.
- **PostgreSQL has no `REPLACE` and quotes `"user"`.** `_meta_set` updates and falls back to an
  INSERT inside its own SAVEPOINT (an IntegrityError in the outer transaction poisons it on
  PostgreSQL), and every statement that names the table is written `"user"` on that engine.
- **A JSON body's playlist id is a string; `Playlist.id` is a UUID.** Comparing them directly
  matched nothing and answered "liste introuvable" for the caller's own playlist — the ids are
  parsed (`uuid.UUID`) before the query, in the same place the regex validates the spelling.
- **A share must not carry another person's private upload.** The test's administrator can see every
  row, so `Track.visible_clause` would have let a friend receive the id of somebody else's file:
  the roster filter is the stricter `owner IS NULL OR owner = <sharer>`, deliberately.
- **The revocation serial moves by itself.** There is no hook in the account manager (a file this
  lot does not own) for "this account was disabled": a digest of the list is stored in `Meta`, and
  the next sync that changes it bumps the serial. A `DELETE` bumps it explicitly.
- **Adding a table that names a user moves the admin deletion budget, but never per row.** Peewee's
  recursive delete emits one DELETE per reverse relation, so the four LAN tables took that test's
  pinned `58 → 65` statements (+7); measured, deleting an account costs **60 statements with 20
  enrolled devices and 60 with none** — the engine's CASCADE removes the rows. The pinned number is
  updated with its reason, never widened.

### LN2 — Trust codec and device key (delivered 2026-11-18)

**What landed.** `cred.rs` and `keys.rs` filled, and `tests/cred.rs` (19 tests) written from the
note's tables rather than from the decoder: the three test vectors are copied into the test with their
layouts and domain strings written out again, so a drift on either side (the Python encoder, this
decoder) breaks it. `Credential::decode/verify`, `Revocations::decode_verify`, `Roster::decode_verify`
and `auth_message` verify over the **exact bytes received**; `DeviceKey::load_or_create/rotate/wipe`
persist a 32-byte seed in `device.key` at `0600` through temp file + `fsync` + rename. Ed25519 and
SHA-256 through `aws-lc-rs` only, **no dependency added** (base64 and hex are decoded in `cred.rs`: the
strictness needed — one spelling of one body — was not something a crate should own), zero `unsafe` in
both files (asserted by a test that reads the sources).

**What a later lot can rely on.**

- `Credential::decode` answers length → magic → version → reserved flags, in that order, before any
  signature work; `verify(keys, revoked, now)` then checks `kid` known → signature → window
  (`not_before − 600 ≤ now ≤ not_after + 600`, saturating arithmetic) → not revoked.
- `Revocations::decode_verify` checks the entry cap **before** computing the length that follows from
  it (a `count` of `0xFFFFFFFF` would otherwise overflow), then the exact length, then the signature,
  then canonical order: the keys must be **strictly ascending**.
- `Roster::raw()` is the signed **body**; `Roster::raw_signed()` is the `{"body","sig"}` envelope, and
  that is what `Control::Roster` carries, base64-encoded. (LN0's comment on `raw()` said "ready to
  gossip"; the body alone is not.)
- `DeviceKey` is the only door to a key, `KEY_FILE` names the file, and `enroll_proof(subject_hex, at)`
  signs `at` as **decimal text** (below).
- Every cause stays local: `CredError` is for the log and the tests, `LanError::Refused` for the wire.

**The numbers, each with its command** (`desktop/`, `CARGO_BUILD_JOBS=4`, debug, 2026-11-18):

| Measure | Value | Command / test |
|---|---|---|
| the package's tests | **63 passed** (44 unit + 19 integration) | `cargo test -p nz-lan --locked` |
| the integration suite | **1.09 s**, including 100 key rotations with `fsync` | `cargo test -p nz-lan --test cred` |
| one-byte mutants of the credential | **154 mutants, 154 refused, 0 accepted** (the intact vector is accepted first, as the control) | `every_single_byte_flip_of_the_vector_is_refused` |
| `CredError` variants | **11 variants, 11 inputs that trigger them** | `every_refusal_variant_has_an_input_that_triggers_it` |
| allocations for 1 MiB refused | **0 bytes** from `decode` and `decode_verify`; the control (a copy of the same buffer) counts **1 048 576** | `decode_allocates_nothing_for_a_bad_length` |
| a decoded credential | **240 bytes** on the stack, **0 bytes** of heap | `a_decoded_credential_is_a_fixed_size_and_allocates_nothing` |
| a revocation list at its cap | **131 169** signed bytes (33 + 32 × 4 096 + 64) + **131 072** for the key array = **262 241 counted bytes** | `an_unsorted_or_duplicated_revocation_list_is_refused` |
| rosters at their caps | **8 083 B** for 64 friends, **180 334 B** for 20 000 track ids, both accepted; 65 friends / 20 001 ids / > 4 MiB → `Caps` | `a_roster_at_its_caps_is_accepted_and_one_over_is_refused` |
| the key file | 32 bytes, `0600`, directory `0700`, one file after 100 rotations, **0 partial reads** | `the_key_file_is_a_0600_seed_created_atomically_rotated_and_wiped`, `a_reader_never_sees_half_a_key_while_it_rotates` |
| lints and format | clippy **0 warning**, rustfmt **0 diff** | `cargo clippy -p nz-lan --all-targets --locked`, `cargo fmt --check` |

Twelve deliberate mutants of the implementation were run once each, and each turned the test that
owns the property red before the file was restored byte for byte: the signature check, the reserved
flags, the revocation lookup, the skew constant (`600` → `599`), the ascending order, the friends'
cap, the roster's "exact bytes" (verifying the base64 instead of the body), the `0600` mode, the
atomic write (delete then recreate), the damaged-key refusal, the revocation cap (`4 096` → `4 097`)
and the role in `auth_message`.

**Rules a measurement taught.**

- **"Verify, then parse" is not implementable for a roster.** The `kid` that names the verifying key
  lives *inside* the body being verified, so the order is: minimal parse of `kid` → verify the exact
  bytes → parse the whole body → caps. What the rule is really after (never build a model from
  unauthenticated bytes) is kept; the literal wording is not, and this section is the correction.
- **A non-canonical revocation list is refused as `CredError::Signature`.** The enum is in this note's
  *Contracts* block, which only the lot that owns the contract may edit, and the semantics fit: a blob
  whose keys are not strictly ascending cannot be the one the Python encoder emitted, so no verifier may
  normalise it into acceptance. `Signature`'s doc says so.
- **`at` in the enrol proof is `str(at)`, decimal.** The formula writes its two neighbours as hex
  strings and writes `i64` wherever it means the binary encoding (`not_before`, `not_after`);
  `the_enroll_proof_signs_the_note_message` verifies the proof over this message **and** asserts that
  the same proof does *not* verify over an 8-byte big-endian `at`, so LN1 and LN7 cannot silently pick
  the other reading. On LN1's request the line is also spelled in *Contracts* (under `keys.rs`), so
  LN7 finds it where it reads the surface.
- **A counting `GlobalAlloc` is the only way to measure "allocates nothing"**, and it costs one
  `unsafe impl` — in the test file. Keeping it there is what lets `cred.rs`/`keys.rs` stay at zero
  `unsafe` and still answer the acceptance with a number instead of an opinion.
- **A restored file keeps its old mtime, and cargo then skips the rebuild.** `cp -p`/`copy2` put the
  backup's timestamp back, so a mutant campaign sees the *mutant's* verdict on a "restored" tree: the
  caps mutant looked green after its revert until every restore was followed by `touch` (measured: the
  restored tree failed `the_caps_are_the_note_numbers` with `4097`). Any mutant script must restore
  **and** touch.
- **The envelope is parsed before the body, so its `sig` length is checked first**: a garbage body with
  a 3-byte signature answers `Length`, not `Json`. The test states that order, because a reviewer will
  otherwise read it as the wrong error.

The note's top *Status* line still says "LN0 delivered, the rest planned"; it belongs to whoever owns
this file's header, not to this lot.


### LN3 — Discovery and the netns harness (delivered 2026-11-18)

**What landed.** `discovery.rs` (the seam, the platform feed, the random names), `discovery_mdns.rs`
(`mdns-sd` browse + register behind the default feature), `tools/netns.sh` and `tests/discovery.rs`.
Nothing is wired into an app yet: `Lan::start` is still LN10's, so a machine with the crate compiled
behaves exactly as before (no thread, no socket — `MdnsDiscovery::new` starts nothing).

- **`Discovery` / `DiscoveryEvent`**: LN0's shape, unchanged. `Found` is ONE address (the shape the
  transport dials); `Lost` names the instance, because that is what a goodbye says.
  `DiscoveryEvent::instance()` is the de-duplication key.
- **`ExternalDiscovery`**: `found`/`lost` become the two events. The platform's FFI carries only
  `(host, port)`, so the instance handle is derived from the address (FNV-1a, 10 hex): the same peer
  keeps the same handle across a re-resolution, a moved peer is a new one, and `lost` names exactly
  what `found` named. `stop()` freezes the feed (a late `NsdManager` callback must not resurrect a
  peer the battery rules stopped looking for) and `start()` unfreezes it. No queue grows without a
  bound: the channel holds 64 events and a flood costs a dropped hint, never a stall.
- **`MdnsDiscovery`**: one daemon, one browse, one register, plus a **plain thread** (`nz-lan-mdns`)
  translating `mdns-sd`'s events — a thread and not a task, so it borrows no runtime and cannot
  outlive one. `stop()` says goodbye, stops the browse, shuts the daemon down and joins both threads;
  `Drop` calls `stop()`; `events()` is taken once and survives a `stop`/`start` cycle (the battery
  rules stop and restart discovery all day, and the consumer must not be left holding a dead channel).
- **The interface policy is two modes, not three.** `new(false)` (the app): every up, non-loopback
  interface. `new(true)` (tests): **loopback only** — not "loopback as well", because a test that
  added loopback would otherwise multicast on the machine's real network (worker rule 8). The harness
  proves the negative half of that every run: the loopback-only discovery it runs alongside never
  sees the announcement made on the dummy interface.
- **Caps, before anything is allocated** (worker rule 10): 8 addresses per resolution (a hostile
  announcement costs at most 8 dials), a 63-byte instance label (a DNS label), a 64-event queue.
  Anything else on the wire is skipped, not parsed further.

**Contracts as they now stand** (LN0's, plus what LN3 added — an addition, never a fork):

```rust
pub const SERVICE_TYPE: &str = "_neezer-lan._udp.local.";
pub const TXT_VERSION_KEY: &str = "v";  pub const TXT_VERSION_VALUE: &str = "1";
pub const INSTANCE_PREFIX: &str = "nz-";
pub fn instance_name() -> String;                                  // "nz-" + 10 hex, OS CSPRNG
impl DiscoveryEvent { pub fn instance(&self) -> &str; }
impl ExternalDiscovery { pub fn is_running(&self) -> bool; }
#[cfg(feature = "mdns")] pub const MDNS_DAEMON_THREAD: &str = "mDNS_daemon";
#[cfg(feature = "mdns")] impl MdnsDiscovery {
    pub fn new(include_loopback: bool) -> Result<Self, LanError>;   // starts nothing; `start` can fail
    pub fn includes_loopback(&self) -> bool;
    pub fn is_running(&self) -> bool;
    pub fn announced_port(&self) -> Option<u16>;
}
```

**The numbers, each with its command** (`desktop/`, `CARGO_BUILD_JOBS=4`, 2026-11-18, debug build):

| Measure | Value | Command |
|---|---|---|
| the crate's unit tests | **52 passed** | `cargo test -p nz-lan --locked --lib` |
| discovery's tests (lib + integration) | **52 + 5 passed** (5 = 4 pure + the harness) | `cargo test -p nz-lan --locked` |
| `browse start → Found` (the acceptance) | **p50 0 ms, p95 0 ms**, 20 runs | `cargo test -p nz-lan --test discovery the_netns_harness_finds_the_other_process -- --exact --nocapture` |
| `start → resolvable by another daemon` | **p50 856 ms, max 1 024 ms** (probing) | same |
| `stop() → the peer's Lost` | **p50 1 000 ms, max 1 001 ms** | same |
| announcement on the wire | instance and host `^nz-[0-9a-f]{10}$`, TXT exactly `v=1`, **20/20 runs** | same (the browser reads the raw record) |
| names re-drawn by `network_changed()` | instance **and** host changed, 20/20 runs | same |
| IPv6 link-local scope kept | `[fe80::1%2]:45940`, scope **2** (the dummy's index), 20/20 runs | same |
| threads after `stop()` and after `Drop` | **0** named `mDNS_daemon`, **0** named `nz-lan-mdns`, 20/20 runs | same |
| a machine without user namespaces | printed `NZLAN HARNESS SKIP …`, exit 0; `NZ_LAN_REQUIRE_NETNS=1` → exit 1 | `PATH=<a fake `unshare`>:$PATH tools/netns.sh --runs 2` |
| the Android shape | **0 warnings** with `--no-default-features`; the roles compile out | `cargo test -p nz-lan --no-default-features --no-run --locked` |
| clippy | green, `-D warnings`, both feature sets | `cargo clippy -p nz-lan --all-targets --locked -- -D warnings` |

**Rules a measurement taught.**

- **An IPv6 multicast packet does not loop on `lo`.** The prototype's recipe (`lo` up + a route for
  224.0.0.0/4) is enough for IPv4 and *silently* useless for IPv6: the same round trip counts
  `Ip6OutMcastPkts 3`, `Ip6InMcastPkts 0` in `/proc/net/snmp6`, so the AAAA records never reach the
  peer and "IPv6 link-local keeps its scope" cannot be measured at all. A **dummy** interface loops
  both families. The harness therefore announces on one dummy interface, which is also the app's real
  policy (`new(false)`) on a machine with one network. *(This is why `tools/netns.sh` no longer
  matches the `lo` sketch this note used to carry.)*
- **A node is not discoverable for ~750 ms after it starts announcing.** `readiness` is not "it
  registered" but "another daemon resolved it": measured p50 856 ms / max 1 024 ms, the three probes
  250 ms apart of RFC 6762 §8.1 plus one resolve. The announcer's `READY` is that event, so the
  latency the harness measures is the *browser's* (0 ms on one interface of one host), not the
  announcer's warm-up.
- **The re-drawn announcement overtakes the goodbye.** After `network_changed()`, a peer receives the
  `Found` of the new name ~300 ms *before* the `Lost` of the old one (mdns-sd emits the removal on its
  next expiry pass). Anything that consumes discovery — this lot's tests, and LN10's connection table
  — must accept the two in either order; waiting for the goodbye first loses the new peer.
- **The goodbye is immediate, the `Lost` is not.** `unregister` broadcasts the TTL-0 records before it
  answers `OK` (and resends once at +120 ms), but the peer emits `Lost` when it next *processes a
  packet*, i.e. at its browse retransmission: measured 1 000–1 001 ms for all 20 runs, against the
  4 500 s PTR TTL a peer would otherwise wait out. The assertion is therefore on the distribution
  (p95 ≤ 1 500 ms), not on a per-run second that the library, not the crate, decides.
- **One resolution is not one event.** `mdns-sd` resolves an instance as soon as its A record is
  known and *again* when the AAAA records arrive, so a peer yields several `Found` for one instance
  (one per address, several times over). Consumers key on `instance()`; a test that assumes a single
  `Found` per peer drops the IPv6 one and "proves" the wrong thing. This was the bug that cost LN3
  the most time.
- **A scope id belongs to a link-local or multicast IPv6 address only** (RFC 4007). `mdns-sd` hands
  the address back with the interface it was *seen* on (`ScopedIp`), and `SocketAddrV6` carries it;
  dropping it produces `fe80::1:45910`, an address that reaches nothing — keeping it on a global
  address is noise and can fail a connect. Measured end to end: `[fe80::1%2]:45940`.
- **AppArmor is the CI's problem, not the developer's.** `unshare -rn true` fails on Ubuntu 24.04
  until `kernel.apparmor_restrict_unprivileged_userns=0`; the workflow step lifts it with one sysctl
  and sets `NZ_LAN_REQUIRE_NETNS=1`, so a runner that loses the ability again turns the job RED
  instead of quietly skipping discovery for months.


### LN4 — Index, hashing, scope, serve (delivered 2026-11-18)

- **Objective** (met): a node knows what it has (downloads + complete play-cache files, per quality,
  with a SHA-256), answers a peer inside that peer's scope and its own serve limits, and never lets
  serving or hashing slow its own playback.
- **Files**: the LN4 row, unchanged — `desktop/crates/nz-lan/src/{store,index,hashing,scope,serve}.rs`
  and `tests/serve.rs`. It changes no shared type and no manifest, so neither `Cargo.lock` moved.
- **What landed.** `lan.sqlite` — WAL, `PRAGMA user_version`, an idempotent v1 plus the boot repair
  path that replays it when a table is missing, a newer file refused with its version in the cause,
  `file_tag`/`peer_addr`/`trust`, the directory `0700` and the file `0600` because a credential lives
  in it. Then the in-memory inventory with a version, a 64-change delta ring and a per-track quality
  bitmask; the `nz-lan-hash` worker; `Scope::for_peer`/`from_share`/`allows`; the responder with its
  serve budget and its single `NotFound` constructor; and 18 acceptance tests that build no `Incoming`
  (LN4's tests drive `Server::answer`, LN6's transport feeds the same function).
- **What a later lot can rely on.**
  - `Store` is now a real handle (`Arc` inside, `Clone`, `Default` = unopened). `Store::tag`,
    `tag_row`, `tags`, `put_tag`, `forget_tag`, `put_trust`/`trust`/`trust_keys`/`forget_trust`,
    `put_peer_addr`/`peer_addrs`/`forget_peers`. An unopened store answers "nothing stored" and writes
    nothing; `Store::db_path`/`dir` answer `None`. LN7's `enroll`/`gossip` and LN3's last-known
    addresses read these and nothing else.
  - `Index::refresh` is the only writer (four statements, whatever the library's size), `events()`
    fires on an inventory change, `sets_version()` moves on a set edit, `local`/`best_local`/`cover`/
    `local_paths`/`statements`/`tracks`/`memory` are the readers, and `hashed()` is the hashing
    progress signal. `refresh` is idempotent: no change, no version bump.
  - `Hashing::start_with(index, store, gates)` is the service's entry point (`start` keeps its declared
    two-argument shape for a node without gates); `state()` reports `Idle`/`Waiting`/`Hashing`.
  - `Server::answer(scope, &Request) -> Answer { head, body, .. }` is the decision,
    `Server::handle` the whole request; the serve slot and the `Bulk` lane are held by the `Answer`
    until the bytes are written, so a peer that hangs up frees them.
  - `Scope::is_own`/`is_all` answer "everything I have" (what a set's own art needs), not "this id".
  - **Hand-offs**: LN6 calls `Server::handle` (which is `answer` + the write, and holds the serve slot
    and the `Bulk` lane across the bytes); LN10 creates the `nz-lan` runtime thread **niced +5** (this
    crate cannot renice a thread it does not create), owns the `Index` (one `refresh` at start, then on
    a timer — `nz-cache` has no "something changed" event — and after a download completes) and starts
    `Hashing::start_with(index, store, api.gates())`, joining it before its runtime stops.
- **Three additions to the note's contract** (each additive, each with its reason):
  1. **`Hashing::start_with`** — `start(index, store)` has no room for the gate, and a worker that
     never sees `Api::gates()` cannot yield. The declared `start` stays; the service uses
     `start_with`.
  2. **`Index::best_local`** and `Fetch`'s fallback to it — the responder answers the asked-for
     quality when it has it and otherwise the best copy it has, and `FetchOk.quality` names what was
     actually served (which is what LN5's `ByteStream::quality` records). `Index::local` stays exact.
  3. **`pub const Store`** — LN0's placeholders in `enroll.rs`/`gossip.rs` use `&Store` as a VALUE
     (a unit struct). A handle cannot be a unit struct, so the same name is also a const: the unopened
     store. Those two files keep compiling untouched until LN7 replaces them.
- **Where a divergence from the plan is deliberate, and why:**
  - **A `Sets` page carries 8 sets, not 64.** The note's 64 was a cap, and it does not fit: 64 sets
    each holding a 4 000-track playlist is ~3 MB of JSON against a 1 MiB frame. Eight worst-case sets
    are 384 KB, and the page also stops at 768 KiB of estimate.
  - **`Meta`'s refusal is `{"t":"Meta","rows":[]}`, not `NotFound`.** A `Meta` request names up to 200
    ids; refusing the whole call for one unshared id would be wrong. The criterion (no information
    from the refusal) holds in the form that matters: the frame for an unshared id and the frame for
    an id nothing knows are byte-identical, which is what the test compares.
  - **A restricted friend gets no set `cover`**, only the art of the sets they can see whole (and of
    the tracks in their scope). `Scope` carries the union of the shared tracks, not playlist ids, so
    "a set they can see whole" is the only rule the signed roster actually supports.
  - **The paused clock is not used**: `start_paused` needs tokio's `test-util`, which this lot's
    manifest does not enable and does not own. The `Bulk` wait is proven by the future's own state
    (one poll ⇒ pending, then completion after the guard is released) and the hashing pause by the
    worker's own `Waiting` event — both event-driven, neither reading the clock.
- **The numbers, each with its command** (`desktop/`, `CARGO_BUILD_JOBS=4`, 2026-11-18):

  | Measure | Value | Command |
  |---|---|---|
  | an 8 000-track cache indexed | **23.8 ms** release, **51.0 ms** debug | `cargo test -p nz-lan --release --test serve the_eight_thousand -- --nocapture` |
  | statements to index it | **4** (downloads, play cache, tags, sets) — the cap is 5 | same run (`Index::statements`) |
  | inventory memory | **48 B/item**, 384 KB for 8 000 tracks (cap 64 B) | same run (`Index::memory`) |
  | hashing throughput | **1 778 MB/s** on 64 MiB (release, SHA-NI) | `cargo test -p nz-lan --release --test serve hashing_reaches -- --nocapture` |
  | hashing niceness | **+10** on `nz-lan-hash`, read back from `/proc/self/task/<tid>/stat` | `cargo test -p nz-lan --lib the_worker_hashes` |
  | the crate's tests | **72 passed** (53 unit + 19 acceptance) | `cargo test -p nz-lan` |
  | a `Bulk` serve that waits for the person | ~1.1 s per test run (the gate's 1 s quiet) | `cargo test -p nz-lan --test serve a_bulk_serve_waits` |
  | the refusal's size | `{"t":"NotFound"}`, 18 bytes, for absent AND unshared | `cargo test -p nz-lan --test serve an_out_of_scope_request` |
  | `clippy -D warnings` on the crate | clean, `--all-targets` | `cargo clippy -p nz-lan --all-targets -- -D warnings` |

- **Mutants, each run once and each RED** (`target/mutants` copies restored after each run):

  | Mutant | Test that caught it |
  |---|---|
  | M1: no scope filter in `Index::inventory` | `a_friend_sees_only_the_tracks_shared_with_them` |
  | M2: out-of-scope `Fetch` answered `Error{code:"scope"}` | `an_out_of_scope_request_is_the_byte_identical_frame_of_an_absent_one` |
  | M3: the hashing worker ignores its gate | `hashing_waits_while_a_user_request_is_in_flight` |
  | M4: a resume onto a different `expect` tag is accepted | `a_fetch_serves_from_the_asked_offset_and_refuses_a_bad_one` |
  | M5: no cover scope check | `a_friend_gets_only_the_art_of_the_sets_they_see_whole` |
  | M6: no serve budget (99 of each) | `the_third_concurrent_play_is_busy_and_the_slot_comes_back` |
  | M7: a `Bulk` serve does not wait on the node's gates | `a_bulk_serve_waits_while_a_user_request_is_in_flight` |
  | M8: a set is advertised whatever its members' scope | `a_friend_sees_only_the_sets_they_can_see_whole` |
  | M9: a file that cannot be read is re-attempted at once | `an_unreadable_file_is_not_retried_in_a_loop` (never reaches `Idle`) |

- **A residual, stated**: a `Cover` answer takes no serve slot and no lane, because the `Cover` frame
  carries no `Urgency` (LN0's shape, final) — the responder cannot tell a list's art from a tap's.
  It is bounded (≤ 2 MiB, and `nz-cache`'s covers are ~50 KB), and LN11 asks them on `Bulk`, so this
  is a requester-side lane rather than a missing server rule.
- **Rules a measurement taught.**
  - **A stored path costs double what the file costs to find.** 8 000 `PathBuf`s (24 B + a ~40-byte
    allocation each) were 90 B/item against the 64 B cap; deriving `nz-cache`'s name from
    `(track, quality)` and keeping only a quality bitmask, four `u32` lengths and the id is 48 B/item
    (23.8 ms release). The derivation is checked, never trusted: `refresh` compares it with the `file`
    column it just read and leaves a mismatch out with one log line, and
    `the_index_derives_the_names_nz_cache_writes` holds it on real files. **Memory is a schema
    decision, not a compaction pass.**
  - **A `u32` length is the right width and the right failure mode**: a file over 4 GiB is refused
    (not truncated), and a track is never that long — while 64 hex characters of SHA-256 per file
    would have been 512 KB more than the whole inventory. The tag is read from `lan.sqlite` on the one
    `Fetch` that needs it (one indexed row).
  - **A gate decides admission, not pre-emption** (`nz_api::lanes`). Holding a `Bulk` slot for a whole
    file is a core kept for its length; taking it per 1 MiB tile is a mutex and a `Notify` per
    megabyte when nobody waits, and a real pause the moment the person taps something — 1 778 MB/s
    measured still, so the yield costs nothing measurable.
  - **`watch::Sender::send` LOSES the value when no receiver is subscribed.** Not "wakes nobody":
    it fails and keeps the previous value, so the first inventory version (and the first hashed
    file) were invisible to every subscriber that arrived afterwards — a peer that connects after a
    refresh would never hear about it, and the CI flake that exposed it was my own test waiting on a
    count that had been thrown away. `send_replace` is the call that stores and wakes; every signal
    this crate publishes (`events`, `hashed`, `attempts`, the hashing state) uses it. The regression
    test is `a_version_bumped_before_any_subscriber_is_still_seen`.
  - **Never wait on a TRANSIENT state after starting a worker.** `Hashing`/`Idle` last microseconds;
    a test that subscribes a tick late waits for ever (2 runs in 5 on a loaded runner, 44 in 60
    pinned to one CPU). A test waits on a MONOTONIC counter (`Hashing::attempts`, or a sticky state)
    that cannot be missed, and proves "no loop" by the worker reaching `Idle` — a spinning worker
    only ever announces `Hashing` again. Verified: 200 runs pinned to one CPU and 200 unpinned, 0
    failures; the mutant fails in every run.
  - **A worker that cannot do a job must remember that.** The first hashing loop re-listed a file it
    had just failed to open (a `chmod 000` copy, a full disk on the tag write) and re-attempted it in a
    tight loop — a core at 100 % for a file nobody can read, the exact thing rule 1 forbids. The
    failure is now remembered per `(len, mtime)`: one attempt, then `Idle`, and a re-download or a
    fixed permission lets it back in. The test waits for `Hashing` then a NEW `Idle`, which a spinning
    worker never reaches (mutant M9).
  - **`Notify::notify_waiters` is not a stop signal.** The window between the worker reading its stop
    flag and registering `notified()` swallows the wake and `stop()` never returns (`cargo test` hung
    for 120 s). `notify_one` stores a permit for the waiter that is not there yet. A test that proves
    "the thread is over" reads a flag the worker sets on its way out, not a `/proc` entry or a channel
    that may still hold an unseen value.
  - **`tokio::time::timeout` must be built INSIDE `block_on`.** Building it first asks the current
    runtime for a timer, and a plain `std::thread` has none: `there is no reactor running` panics the
    hashing worker (and the same mistake in a test). Every timer in a thread that owns its runtime is
    created inside the `block_on` that awaits it.
  - **A serve slot is owned by the answer, so a test that reads two answers holds two slots** — the
    third `Play` of a naive test is `Busy`, not a bug. Freed by dropping the `Answer` (or its `head`),
    which is also what makes "a peer hangs up" free its slot.
  - **`Meta`'s metadata is not in memory**: the album, its id, the duration, the art and `explicit`
    come from the `/playlist/<id>` and `/album/<id>` bodies the set pages left in the API cache, read
    for the ≤ 200 ids a request names (one `download_members` query says which sets to parse). Titles
    and artists come from `nz-offline`'s `downloads.labels`. A track no body knows answers its labels
    only — never a guessed album.
  - **A page bound that is computed from what a page HAPPENS to send skips sets.** The first `Sets`
    page builder stopped at 768 KiB of estimated JSON and answered `last = false`; the peer then asked
    page+1, which starts at `page × PAGE_SETS` — so every set after the cut was lost, silently. The
    bound is now computed from the set list's own `tracks` counts (one statement, no member read) into
    deterministic page slices, with a 43-byte-per-id estimate and 8 sets per page: a page that passes
    it is under 512 KB of JSON, no set is ever skipped, and a page past the end answers "nothing
    follows" instead of inviting one more request for ever.
  - **A restricted friend's covers are built once per (scope, sets version)**, from the sets whose
    members are all in scope: walking their 20 000 ids per cover request would let a peer spend this
    device's CPU by asking in a loop, and the set of cover URLs is a few hundred at most.


### LN5 — Shared-crate adaptations (delivered 2026-11-18)

**What landed.** A source now says which quality it REALLY serves, and both consumers of bytes
record that instead of the rung they asked for: `nz_audio::ByteStream::quality() -> Option<Quality>`
(default `None`), `EngineEvent::StreamComplete` carrying `stream.quality().unwrap_or(requested)`,
`nz_offline::Fetched { bytes, quality }` with `Downloader::fetch -> Result<Fetched, Outcome>`, and
`nz_cache::Downloads::complete_as(owner, track, bytes, quality)`; `complete` keeps its signature and
calls it with the reserved quality. No `Cargo.toml` and **no `Cargo.lock`** line moved in either
workspace, and neither app needed a behaviour edit: `StreamComplete`'s `quality` is still
`nz_audio::Quality`, so the desktop's `state.rs`/`publish_spool` and the phone's `nz-core::player`
destructure it unchanged, and every other `ByteStream` (`ApiByteStream`, `MeasuredStream`,
`HttpBody`, `FileSource`) keeps the default `None` — which IS the server's behaviour.

- **The rule, a file's quality is the quality it IS.** The name, the row and the label a screen
  prints are the digest and the word of `(track, quality really received)`, never of the request. A
  peer that holds only the FLAC answers an Opus 320 download in FLAC; publishing it as
  `{digest}.OPUS_320` would make the store, the engine's `local()` seam and LN4's LAN index claim a
  transcode that never existed, and a later `local()` would answer for a rung whose file is not on
  disk. Measured: reserved at `OPUS_320`, delivered `FLAC` → **1** file `<digest(FLAC)>.FLAC`,
  **0** `.part`, `Downloads::local` = `(that file, Flac)`, `quality_label` = « FLAC »
  (`cargo test -p nz-cache --lib a_download_delivered_in_flac`,
  `cargo test -p nz-offline --lib a_download_delivered_in_flac`).
- **The rule, `complete` is `complete_as` at the reserved quality.** One implementation
  (`Downloads::publish`) plus a rename, so the server's path cannot drift from a peer's: the
  reserved `.part` is renamed, never trusted before, and the length is read back from the published
  file. One read + one `UPDATE`, as before.
- **The rule, the engine's spool keeps the REQUESTED name while the event tells the truth.** The
  spool is private scratch keyed by `(track, requested quality)`; naming it by the delivered quality
  would collide with a concurrent voice spooling the same track at that quality, and only the cache
  row matters. So `HttpSource` names the spool exactly as before and carries
  `delivered = body.quality().unwrap_or(requested)` into `StreamComplete`; the app's `publish_spool`
  (unchanged) reserves the play cache at THAT quality, which is what makes the entry
  `(track, FLAC)`. Measured: a 4 096-byte body reporting FLAC for an `OPUS_320` request → the event
  says `Flac` (`cargo test -p nz-audio --lib
  the_completion_event_carries_the_quality_the_body_really_served`) and the play cache holds
  `(track, FLAC)` and **nothing** under `OPUS_320` (`cargo test -p neezer-desktop --bins
  wiring::engine::tests::a_spool_announced_as_flac`).
- **The rule, the phone needed zero lines.** `StreamComplete.quality` stays `nz_audio::Quality`, and
  `nz-core`'s `MeasuredStream` is a LEAF body (it owns the server's response), not a wrapper — so
  the default `None` is right for it, and LN13's `PeerFirstOpener` will report a peer's quality from
  `nz-lan`'s own `ByteStream` instead. `mobile/rust`'s whole suite stayed green with no edit.
- **The rule, each new assertion was first run against a broken build.** Three deliberate mutants,
  each one refused by the test that names it: `delivered = quality` in `HttpSource::new` (the
  engine's test fails), `Downloads::complete_as` forwarding the RESERVED quality (the store's test
  fails), and the worker calling `complete` instead of `complete_as` (the worker's test fails).
- **Stated for LN11/LN12, not solved here:** a play cached under FLAC is not found by a later
  `OPUS_320` request — `opener.local(id, Opus320)` only asks that rung — so the opener decides again
  and may re-fetch from the peer. Nothing lies on disk; a later lot can cheapen that second play by
  asking `local()` at the tolerated qualities `decide` already ranks.

### LN6 — Transport and authentication (delivered 2026-11-18)

**What landed.** `codec.rs`, `auth.rs` and `transport.rs` filled, and `tests/transport.rs` — 20 tests
written from this note's *Acceptance* list rather than from the implementation: two real `Endpoint`s
on loopback, each with its own key written on disk under `CARGO_TARGET_TMPDIR` (never `/tmp`) and its
own credential signed by a test server key, plus a **raw quinn client** for every hostile case (TLS is
not the identity, so an attacker's test has to get through it). `nz-lan` gains one dev-dependency
(`tokio` with `test-util`, for the paused clock); **neither `Cargo.lock` changes**, because that
package and version were already in both.

**What a later lot can rely on.**

- `Endpoint::bind(rt, port, id, trust, hello)` builds the ephemeral Ed25519 certificate, both TLS
  configurations and a **dual-stack** UDP socket; a taken preferred port becomes an ephemeral one and
  `port_fallback()` says so. `set_identity` swaps a renewed key; a live connection keeps the identity
  it authenticated with.
- `connect` answers only once **both** proofs and **both** `Hello`s are done. `PeerConn::info()` then
  carries `subject`, `device_pub`, `relation`, `scope`, `hello` and `addr` (a v4-mapped IPv6 peer is
  stored as the IPv4 a router shows).
- `accept()` answers the connection that survives **even when the pair already had one**, `Err(Refused)`
  for a stranger, `Err(Proto)` for a major this build cannot speak, `None` once the endpoint is closed.
- `PeerConn`: `request` → `Reply`/`Body::chunk` (≤ 64 KiB whatever the caller asks, clamped by the
  announced length), `next_incoming` → `Incoming::respond` (the head's announced length is enforced),
  `send_control`/`next_control`, `rtt`, `dialled_by_us`, `close(code)`, and `connection()` — the last
  one only for the tests and a log.
- **Three** application close codes: `0x10` refused (`auth::CLOSE_REFUSED`), `0x11` wrong major
  (`auth::CLOSE_WRONG_MAJOR`), and `0x12` **duplicate** (`transport::CLOSE_DUPLICATE`, new). A
  duplicate is not a refusal: a refusal means "do not dial me again for ten minutes", and the pair is
  already connected.
- `PeerInfo.scope` is the **empty friend scope** when I dialled a peer that shares with me but to whom
  I share nothing: `Scope` has no third variant, and an empty `Friend` answers `NotFound` to every
  request — the answer an absent file gets (T7).
- **No keep-alive by default**: an idle connection closes after 30 s and is re-dialled
  (one handshake, ~3 ms) when the person comes back. The note's 10 s keep-alive is a
  *foreground* policy and quinn only takes the interval at configuration time, so
  enabling it inside the endpoint would ping every ten seconds with the screen off —
  exactly what the battery measure forbids ("0 LAN packets"). A transfer keeps itself
  alive by sending bytes.
- `codec::frame_body_len(header, cap)` is the pure "check before allocate" decision;
  `codec::MAX_CHUNK_BYTES` is 64 KiB. `Incoming` and `Body` own their stream, so a caller never sees a
  `quinn` type.

**The numbers, each with its command** (`desktop/`, `CARGO_BUILD_JOBS=4`, 2026-11-18):

| Measure | Value | Command / test |
|---|---|---|
| LN6's suite | **20 tests**, 5.2 s | `cargo test -p nz-lan --test transport` |
| handshake + both proofs, loopback, 100 runs | **release: p50 0.63–0.90 ms, p95 1.00–1.35 ms** · debug: p50 3.3–6.2 ms, p95 4.0–9.0 ms (two runs each; the debug spread is this machine's load, which is why the debug assertion is a 1 s ceiling and the note's number is the release one) | `cargo test -p nz-lan [--release] --test transport the_handshake_and_both_proofs_are_fast -- --nocapture` |
| a stranger's answer | **0 bytes**, one code, empty reason | `a_stranger_gets_no_identity_byte` |
| one refusal, five causes | 5 causes → **1 code (`0x10`), 0 bytes each** | `every_refusal_is_the_same_code_and_no_identity_byte` |
| the pre-auth budget | refused at **exactly 2 s** of virtual time, and on a header over **2 KiB** (0 s spent) | `the_responder_closes_two_seconds_without_a_valid_auth`, `the_responder_refuses_more_than_two_kib_before_auth` |
| an over-cap frame | 1 MiB + 1 refused; the decision costs the **same** for 1 MiB + 1, 8 MiB and 4 GiB | `an_over_cap_frame_allocates_nothing_and_stays_under_a_mebibyte` |
| 0-RTT | `max_early_data_size = 0`, `enable_early_data = false`, a greedy client resumes nothing | `early_data_is_never_accepted` |
| TLS 1.2 | refused, `PeerIncompatible(Tls12NotOfferedOrEnabled)`; the TLS 1.3 control completes | `tls12_cannot_establish_a_session` |
| duplicates | **1** connection per pair on both sides; the lower `device_pub`'s dial survives, proven by a control frame | `a_duplicate_pair_collapses_to_the_lower_key_dial` |
| the connection cap | **16** held, the 17th refused, the table does not grow | `the_connection_cap_is_sixteen` |
| tasks | **0** `tokio::spawn` in the three files; `close` joins quinn's driver and `accept` then answers at once | `the_transport_spawns_no_task_of_its_own`, `closing_the_endpoint_joins_the_only_task` |
| the port fallback | taken port → ephemeral, `port_fallback() == true` | `a_busy_preferred_port_falls_back_to_an_ephemeral_one` |
| both locks | **unchanged** | `git diff --stat -- desktop/Cargo.lock mobile/rust/Cargo.lock` (empty) |

**17 deliberate mutants**, one per security property, each run once against the broken code and then
against the restored file (restore **and touch**: without the touch cargo keeps the mutant's verdict —
LN2's lesson). **17 killed, 0 survived**, every control green: cap-before-allocation, no cap at all, a
`Hello` written before the proof, one code per cause, the wrong role in the proof (reflection
accepted), a constant exporter (replay accepted), an initiator that accepts any responder, a responder
authorising from `trusted_by`, a 60 s pre-auth deadline, a 1 MiB pre-auth budget, 0-RTT enabled, TLS
1.2 accepted, the duplicate rule inverted, 18 connections, `close` not closing, `close` not joining,
and an empty `Hello`.

**Rules a measurement taught.**

- **A raw quinn client cannot finish a handshake unless the responder's `accept()` is polled.** Six
  hostile tests timed out at `CONNECT_DEADLINE` (5 s each) until every one started its dial *inside*
  the same `join!` as the accept. A test that dials and then accepts deadlocks, and the symptom looks
  like a transport bug.
- **`accept()` must answer the connection the pair already had.** "A new one only" made the duplicate
  test wait for ever: one side's incoming loses the race, the caller is never told, and the peer it
  dialled never appears. The survivor IS the event.
- **The connection cap has to be checked before the `AuthOk` is written.** Checked only at insert
  time, the seventeenth peer completed its own half and held a connection the responder had already
  closed (measured: `connect` answered `Ok`).
- **A refusal cannot be observed from the other side of the connection.** When the responder leaks a
  frame and closes in the same breath, QUIC drops what was queued — the peer counts 0 bytes either
  way. The property is therefore asserted **on the source**: inside `accept_one` the first
  `codec::write_frame` comes after `auth::verify_peer`
  (`a_responder_writes_nothing_before_it_verified_the_peer`), and that scan is what killed the mutant.
- **`start_paused` advances the clock on its own when the runtime idles**, so "not finished yet" is
  not an assertion: it drifts to the next timer. The pre-auth deadline is asserted on *where the
  virtual clock lands* — exactly `PREAUTH_DEADLINE` — and the over-2 KiB refusal on **zero** virtual
  time spent, which is what tells a header-only refusal from one that waited for the body.
- **The note's auth message is 59 bytes, not 58**: `b"neezer-lan/1 auth\0"` is 18 bytes (17 characters
  plus the NUL), so 18 + 9 + 32. LN2's `auth_message` is the implementation and its vector is what LN1
  signs; the prose was off by one, not the code.
- **"Allocates nothing" is really "allocates nothing proportional".** Refusing a frame builds an
  80-byte error sentence, so the honest assertion is that the cost is *identical* for 1 MiB + 1, 8 MiB
  and 4 GiB (`assert_eq` of the three deltas), never that it is zero.
- **TLS 1.2 is refused by the server before any certificate work**, and the client reports
  `PeerIncompatible(Tls12NotOfferedOrEnabled)` — not an alert, which is what "TLS 1.2 refused" invites
  you to assert. The TLS 1.3 control (both configurations, pumped through two in-memory buffers) is
  what proves the failure is the version.
- **quinn 0.11.12 has no public `accepted_0rtt()`** (it lives on `quinn_proto::Connection`): the 0-RTT
  property is asserted on the three public things that decide it — the server's
  `max_early_data_size == 0`, the client's `enable_early_data == false`, and a client that asks for
  early data never getting a session accepted.
- **A dual-stack listener needs five `unsafe` calls.** `std` can neither create-then-bind a socket with
  `IPV6_V6ONLY = 0` nor turn the option off after `bind`, and `socket2` is not a dependency here; the
  five `libc` blocks live in `dual_stack` alone, each with its SAFETY comment, and a source scan keeps
  them there (and fails if the count moves).
- **Every test needs a bound, and so does every test command.** A mutant that removes `close`'s work
  left `endpoint.accept()` waiting for a connection that could never come: the test hung for 41
  minutes at 0 % CPU instead of failing. `bounded(...)` (a 60 s net, with the test's name in the
  panic) now wraps every test body, and the mutant campaign's `cargo test` runs are wrapped in a
  180 s `timeout`.

### LN7 — Enrolment and trust sync (delivered 2026-11-18)

**What landed.** The device side of the trust chain, and the twelve `nz-api` bodies.

- **`nz-lan/src/enroll.rs`** — `ensure_enrolled`, `sync_trust`, `forget`, `load_cached`, plus the
  store's five trust rows, the ETag derivation and the base64/hex decoders with their caps.
- **`nz-lan/src/gossip.rs`** — `offers` / `accept`: one `Revocations` per `kid` this device is ahead
  on, one `Roster` for one's own subject; nothing unverified is stored, nothing over 1 MiB of base64
  is decoded, and a lower serial is silence (`Ok(false)`), not an error.
- **`nz-lan/src/settings.rs`** — the four `lan.*` keys in `nz-cache`'s `kv`, with the defaults of the
  note and rule 7's fallback (a row that no longer parses reads as its default; an empty or unknown
  tolerated list is the default set, because an empty one means "never take anything from a peer").
- **`nz-api/src/endpoints/lan.rs`** — the twelve bodies on LN0's lanes; **`nz-api/src/client.rs`**
  gained `get_conditional` (the one shared-crate addition, approved by the orchestrator; see *Rules*).
- **`nz-lan/tests/enroll.rs`** — 23 tests against a fake HTTP server that signs LN1's contract with
  the note's LAN seed and asserts what the WIRE carried (see the numbers).

**What a later lot can rely on.**

- `ensure_enrolled` answers from disk with **0 requests** while the credential has more than 23 days
  left; otherwise `GET /api/lan/status` → key → `POST /api/lan/enroll`. A renewal **rotates the key
  and keeps the device row** (`device_id` read from the stored credential), so a device never takes a
  second slot under the 20-device cap. A `404` on status is `Unsupported`, `enabled=false` is
  `Disabled`, and **neither writes anything**: no key, no `lan.sqlite`, the directory stays empty.
- **A refused renewal keeps the device a node.** A renewal stages its next key in `staged-key/` and
  only renames it onto `device.key` once the credential, the roster and the revocation list have all
  verified: a `403`, a lost answer or a crash leaves the old key with the credential it still holds,
  and the next contact renews on the same `device_id`. Nothing is left behind on the failure path.
- `sync_trust` is one `Bulk` conditional GET; `Ok(false)` on `304`. It verifies both blobs against
  the `ca:` keys the store holds, and refuses a roster about another subject with `Mismatch`.
- `forget` wipes the `trust` rows **first**, then `device.key` (so a reader never sees a key without
  a credential), then `DELETE /api/lan/devices/self` when the client says it is online; a failed
  revoke is a `debug!`, because the local wipe is what makes the device unusable.
- `load_cached` never writes: a credential in the store with no key file answers `None` and mints no
  key, and an expired or revoked credential answers `None` too (`Expired` is not "load it anyway").
- The trust rows are `credential` (154 raw bytes), `roster` (the `{"body","sig"}` envelope — what
  gossip relays), `revocations:<kid>` (raw, what gossip relays), `ca:<kid>` (the 32-byte Ed25519 key)
  and **`roster.etag`**, one key the note's *Device storage* list had no place for: a blob key, no
  schema change, wiped by `forget` with the rest.
- `stale_proof` (the server's `403`) becomes `LanError::Server` whose `cause_fr` says « l'heure de
  l'appareil est décalée » — the one refusal a person can act on. Every other cause keeps the
  server's own French sentence.
- Success is silent (`debug!`); `warn!` is only a refusal or a corrupted row, and **no key, credential,
  roster, revocation list or cookie ever reaches a log line** — the blob is named by its KEY.

**The numbers, each with its command** (`desktop/`, `CARGO_BUILD_JOBS=4`, debug, 2026-11-18):

| Measure | Value | Command / test |
|---|---|---|
| First login | **3 requests** (probe `OPTIONS`, `GET status`, `POST enroll`), **242 B** of request bodies + **1 217 B** of answers = **1 459 B** | `cargo test -p nz-lan --test enroll first_login -- --nocapture` |
| Second start within 24 h | **0 requests**, same credential, same key file | `a_second_start_within_a_day_makes_zero_requests` |
| Daily sync, nothing moved | **1 request**, `If-None-Match: "42-7"`, `304`, **0 body bytes** | `a_day_later_one_conditional_roster_request_answers_304` |
| Daily sync, roster moved | 1 request, `200`, new validator `"43-7"` stored, next sync a `304` | `a_roster_that_moved_is_fetched_verified_and_stored` |
| Renewal threshold | `> 23 d` → 0 requests; `= 23 d` → **2 requests** (status + enrol), new key, same `device_id` | `a_credential_with_less_than_23_days_left_renews_with_a_new_key` |
| Server without the route / feature off | `Unsupported` / `Disabled`, **0 file** in the data directory | `a_server_without_the_route_writes_nothing`, `a_disabled_feature_writes_nothing` |
| Offline start | credential + roster + revocation list loaded, **0 requests**, `None` past `not_after` | `an_offline_start_reads_the_cached_credential_and_trust` |
| Enrol proof | verifies over `subject_hex ‖ pubkey_hex ‖ str(at)`; **refused** over `at.to_be_bytes()` | `the_enrol_proof_binds_the_subject_the_key_and_the_decimal_instant` |
| Logout | `DELETE /devices/self` (1 request, no `X-NS-Background` = `User`), key gone, 0 trust rows; offline: 0 requests, same wipe | `forget_wipes_everything_and_revokes_once` |
| Gossip | higher serial stored byte for byte and **used** (`load_cached` refuses the revoked device); equal/lower/unknown-kid/forged/over-cap all refused | `gossip_takes_a_higher_revocation_list_and_uses_it` |
| Mutants | **24 mutants, 24 refused**, each test red against deliberately broken code, file restored + `touch`ed | `python3 target/ln7-mutants.py` |
| A refused renewal | the old key file survives byte for byte, no `staged-key/` left, `load_cached` still answers; the retry rotates on the same `device_id` | `a_refused_renewal_keeps_the_device_a_node` |
| The crate's tests | **67 unit + 19 cred + 23 enroll + 18 serve**, all green | `cargo test -p nz-lan --locked` |
| `nz-api` (client change) | **42 unit + 6 bench + 22 others** green, bench against the real Flask server | `cargo test -p nz-api --locked` (`NZ_PYTHON=…`) |
| The contract against the REAL server | derived `ETag "0-0"` → **304, 0 B**; unconditional roster **502 B**; `DELETE …/self` → `204` then the row is revoked | `desktop/target/ln7-live-check.py` (working material, not committed) |
| Lints, both workspaces | desktop clippy `-D warnings` clean, rustfmt clean; `mobile/rust` `cargo check/clippy --workspace --locked` clean; **neither `Cargo.lock` changed** | the CI commands |

**Two facts outside this lot's file list, both declared in the PR.** `nz-api/src/client.rs` gained
`get_conditional` (the roster's `304`; `Api` had no way to send `If-None-Match` and `classify` treats
`304` as an error — the orchestrator approved the additive change). And the `roster.etag` row above.

**Rules a measurement taught.**

- **The ETag is derivable, so the first daily sync is already conditional.** `ETag` is
  `"<roster serial>-<revocation serial>"` and an enrolment carries both numbers: deriving it costs
  nothing, while asking for it costs one full roster per device per day. Proven live: derived
  `"0-0"` → `304`, 0 bytes, against `neezer/webui/lan.py`.
- **A fake server records the hit BEFORE it answers.** `tests/enroll.rs`'s fake wrote its answer
  and then pushed the `Hit` into the shared log, so a client that already had the response could
  look before the log was updated: `a_day_later_one_conditional_roster_request_answers_304` then read
  `hits.len()` as **0** for a request that DID happen. Measured: **1 of 200** turns unfixed on an
  idle machine, **4 of 200** pinned to one core (`taskset -c 0`), and the CI job that failed on
  another PR with 22 of 23. Recording first makes the hit happen-before the response, which is the
  only order where the log a client sees is the log it caused: **200/200** and **200/200** after.
  No injectable clock or ETag was involved — the daily sync under test was correct.
- **A test must not compute its boundary from the constant it is testing.** The renewal test's first
  version used `RENEW_BELOW_SECONDS` for "one second above / exactly at" — the mutant that moved the
  constant by one second **survived**, because the boundary moved with it. The test now spells
  `23 * DAY` and asserts the constant, and the mutant dies. Two other survivors (an untested
  key-creation guard, a missing roster-subject test) were found the same way: a mutant campaign that
  finishes green on the first try has not tested anything.
- **Rotating a key "in place" is a rule-3 bug, and only a refusal test finds it.** The first version
  called `DeviceKey::rotate(dir)` before the `POST`: when the server then refused, the stored
  credential no longer matched the key on disk, so `load_cached` answered `None` and a device with
  23 days of validity left stopped being a node. The key is now staged in `staged-key/` and promoted
  by one `rename` after everything verified; the mutant that rotates in place fails
  `a_refused_renewal_keeps_the_device_a_node`, and so does the one that drops the staging guard.
- **A sync never moves the device backwards.** The server is authoritative, but an answer with a
  serial BELOW what this device holds (a proxy replaying an old body) is not stored: only the
  validator is taken, so the next request is conditional on the server's own current answer. The
  decision and the `put_trust` happen under one write lock, which is what stops a concurrent gossip
  from being overwritten; the mutant that drops the comparison (always "moved") turns
  `an_older_roster_never_moves_the_device_backwards` red.
- **`Store` has no per-key delete** (LN4's file), so a gossiped roster leaves `roster.etag` alone:
  the next sync sends a stale validator, the server answers one full roster, and the fresh validator
  is stored. Never a wrong roster, at most one extra body — cheaper than a second file's method.
- **Opening `lan.sqlite` creates it**, so "a server without the feature writes nothing" needs the
  open to happen *after* the status answer and only when the file already exists. `DeviceKey` has
  only `load_or_create`, so "a read never creates a key" is a `KEY_FILE.is_file()` guard placed
  *after* the credential check; both are asserted with a mutant that removes them.
- **`at` in the enrol proof is `str(at)`, decimal** — LN2 said it, and the wire test now proves it
  from the other side: the proof verifies over the decimal message and does not verify over
  `at.to_be_bytes()`, so the Python server and this client cannot drift apart silently.
- **A blob from the network is capped before it is decoded.** A revocation list at its 4 096-key cap
  is 174 892 base64 characters; `accept` refuses anything over 1 MiB of text *without* allocating the
  decoded body (mutant: drop the cap → the unit test's `CredError::Caps` becomes a decode error).

### LN8 — Web UI « Appareils et amis » (delivered 2026-11-18)

**What landed.** Réglages → « Appareils et amis », the one place a person acts on the LAN trust
chain from a browser. Three blocks in one category: **Mes appareils** (name, platform, enrolment
date, last seen, « cet appareil », rename, « Révoquer » behind a second tap), **Amis de confiance**
(add by account name, « Tout partager » or a playlist picker, remove, plus the read-only « Qui
partage avec vous »), and — administrators only — **Serveur**: the feature switch, the credential
lifetime (7–90 days) and the server key's id with its renewal behind a second tap. Every mutation is
optimistic, and every refusal puts the row back with its cause and « Réessayer ».

- `webapp/src/lib/lan.js` — the routes of this note's *Server* table and the list edits the screen
  rolls back with, in one place with no DOM in it, so `node --test` covers the wire and the words:
  `lanFeature()` (the probe), `devices/friends/settings`, `renameDevice/revokeDevice`,
  `addFriend/setFriendScope/removeFriend`, `saveSettings/renewKey`, `playlistChoices()`, and the
  pure `renameDeviceIn/withoutDevice/withScope/withoutFriend/togglePlaylist/sharedPairs`, plus the
  French words (`platformLabel`, `seenLabel`, `deviceDetail`, `sharedSummary`, `daysLabel`,
  `keyLabel`).
- `webapp/src/components/LanSettings.svelte` — the three cards. `LoadFailed` + « Réessayer » for the
  section's own load; a cause line per row for a failed mutation; `role="switch"` with
  `aria-checked` on the two switches; every icon-only control labelled; glyphs through `Icon.svelte`;
  no emoji; the design system's `.card`, `.btn`, `.field`, `.chip` and a switch drawn the way
  `webapp.md` draws one (`--btn`/`--onbtn` track, `--s2` + `--dim` when off). The device glyph is the
  platform's: a screen for a computer, a handset for Android and iOS (`smartphone` was added to
  `Icon.svelte` — no existing glyph said "phone", and `cast` said the wrong thing).
- `webapp/src/routes/Settings.svelte` — one import, one `onMount` probe, one nav entry in the
  « Comptes & serveur » group and one `{#if tab === "lan"}` include. Nothing else.
- `webapp/test/lan.test.mjs` (13 tests) and the screenshots under
  `docs/agents/screenshots/lan/LN8/` (desktop, the picker, the server card, a phone at 390 px, the
  player's 340 px panel width, and a server without the feature).

**Two corrections to this plan, taken before merge.**

1. **The test is `webapp/test/lan.test.mjs`, not `lan.test.js`.** `npm test` is
   `node --test "test/*.test.mjs"`: a `.js` file at that path is never picked up, so the lot's whole
   proof would have been dead weight in CI.
2. **`enabled=false` must NOT hide the screen; only a server without the routes does.** The *Server*
   table says « `enabled=false` hides the feature in every client », which is right for an app's
   node. Applied to Réglages it contradicts rule 6: the administrator would have no door left to
   switch the exchange back on. So the probe hides the category when `/api/lan/status` is not a 200
   (an older server, or a session the routes are `NEVER` for), and `enabled=false` draws the section
   with a banner that says so — the admin re-enables it from the « Serveur » card, everybody else is
   told to ask them.

**What a later lot can rely on.** `lanFeature()` answers `{supported, enabled, credential_days,
keys, subject}` and remembers a DEFINITE answer for the page load only (a 404 is cached, a network
failure is not, so a later Réglages visit tries again). `devices()` is the bare array of the route,
`friends()` the `{shares_to, trusted_by}` object with `user_id` as the account UUID and `subject`
separate, `settings()` `{enabled, credential_days}`; a refusal is `{status, message, code}` and
`lan.cause()` is the same `failureCause` every other screen shows, so « HTTP 404 : Aucun compte de ce
nom. » reads alike everywhere. `playlistChoices()` reads `/me/playlists` from the SPA's offline cache
first and costs a request only on a cold one.

**The numbers, each with its command** (this machine, `npm run build` before the bench,
`~/.cache/ln8/bench.mjs` on `tools/perf_api.py --serve --tracks 400 --admin-users 5`, real Ed25519
enrolments and real friendships through the routes; `PYTHON=~/.cache/neezer-ln8/venv/bin/python`):

| Measure | Value | Where |
|---|---|---|
| Opening the section, admin | **3 requests** (`/api/lan/devices`, `/api/lan/friends`, `/api/lan/settings`) | bench, counted on `response` events under `/api/lan/` |
| Opening it, non-admin | **2 requests** (devices, friends); **0** to `/api/lan/settings` | same bench, as `membre` |
| The feature probe | **1 request per page load**, **0** on a second visit to the category | bench (`probe=1`, then 3 and only 3) |
| Before the category is opened | **0** `/api/lan/*` requests | bench |
| A server without the routes | `/api/lan/status` 404 → **0 further LAN requests**, **0** categories drawn | bench, status route answered 404 |
| Rename, optimistic | new name painted **while its PATCH is held open** (0 console/frame work); on a 500 the old name returns with « erreur du serveur (HTTP 500 : Erreur serveur.) » and « Réessayer » | bench |
| Friend picker | **0** requests on a cached library; **1 GET** cold | bench + `lan.test.mjs` |
| Layout | **0 px** sideways overflow at 390 and 340 px; **0** control wider than the column; device rows stack their actions under the name | bench (`scrollWidth - clientWidth`, then a per-element right edge) |
| The card's inner gabarit | **22 px** from the border to the title in all three cards (Réglages' own padding) | bench, `title.left - card.left - 1` |
| The device line | **0** meta segment cut across lines; **3 lines** of explanation before the first device at 340 px (4 before this pass) | bench, `getClientRects().length > 1` per segment, then the paragraph's height / line-height |
| The last card under the mini player | ends **32 px above** it on a phone (676 ≤ 708) and **28 px** at 340 px (636 ≤ 664) | bench, `scrollIntoViewIfNeeded` then the two rects |
| `npm test` | **804 passed, 0 failed** (13 new) | `cd webapp && npm test` |
| `npm run build` | green, **0 warning** naming `LanSettings.svelte` or `lib/lan.js` | `cd webapp && npm run build` |

**Rules a measurement taught.**

- **A child component's `.card` inherits NOTHING from the screen that renders it.** Svelte scopes
  CSS per component, so `Settings.svelte`'s `.card` (22 px of padding, 16 px/600 title, 0.85 rem
  subtitle) never reached `LanSettings.svelte`'s sections: measured on the first phone capture, the
  title and the paragraph sat **flush against the card's border (0 px)** while the rows were
  indented. The gabarit has to be repeated in the card's own component — same numbers, same tokens —
  and the bench now asserts `22` on every card.
- **A line of facts is a row of unbreakable segments, not a sentence.** "vu il y a 1 min" came out as
  "vu il y / a 1 min" at 340 px. Each fact is its own nowrap span and the line wraps between them
  (`display: flex; flex-wrap: wrap`), the separator glued to the NEXT segment so it can never end a
  line alone; and the enrolment date — the least useful of the three on a phone — is the fact a
  narrow column drops (`@media (max-width: 760px)`), which took the paragraph above the first device
  from 4 lines to 3.
- **A `<button>` laid out with `display: flex` will not shrink its text.** The admin switch's
  explanation and its title stayed on ONE line at 1280 px, the hint's last line running past the
  card's content box: `justify-content: space-between` gave the text column its max-content width and
  pushed the 44 px knob out. `display: grid; grid-template-columns: minmax(0, 1fr) auto` is what
  constrains it (measured in the built app: `690px 44px` inside a 750 px row, the hint on its own
  line). Text beside a fixed control is a grid, not a flex row that is hoped to wrap.
- **The friend picker must `peek`, not `swr`.** `api.swr("/me/playlists")` always fires a network
  call; the 0-request picker comes from `api.peek()` (a cache read that returns parsed JSON) with one
  `api.get()` only when the cache is empty. `isCacheable` lists no `/lan/*` prefix, so no trust
  payload ever lands in the device's offline copy.
- **A refusal the server words itself is the cause to show.** `failureCause` composes
  `HTTP <status> : <server's French sentence>`, which is what the person needs (« Aucun compte de ce
  nom. », « Trop d'appareils enrôlés… »); a row-level error line is enough, and the destructive
  actions need a second tap, not a red colour — this design system has none, and an accented
  « Révoquer » read as the row's primary action.
- **Only the CONFIRM step is accented.** The first « Révoquer »/« Retirer » is an ordinary ghost
  button; the confirmation is the one that carries `--ac-text`. Colour is not the carrier of meaning
  (§10) and the word is.
- **The SPA scrolls inside its own box, so `fullPage: true` captures the viewport.** A screenshot
  taken after clicking a picker showed the middle of the page with the header clipped. Move the mouse
  into the content and wheel up (or zero every `scrollTop`) before the shot, and `scrollIntoViewIfNeeded`
  for a shot of a block further down; the notice strip above the content is dismissed with its own
  « Masquer » button rather than removed from the DOM.
- **Bench a row that swaps its text for an input by INDEX.** « Renommer » replaces the name span with
  a field, so `filter({ hasText: "Bureau" })` stops matching the moment the field opens; the row is
  found by its position, then `nth()` holds for the assertion and for the rollback.
- **Count only the requests being measured.** `page.on("response")` over everything counts the SPA's
  own shell and `lib/net.js`'s `/api/me` probe; the numbers above filter on the `/api/lan/` prefix
  (plus `/api/me/playlists` for the picker).
- **`startServer()` returns `stop`, and it is not `stopServer`.** `await stopServer(stop)` takes the
  returned *function* as if it were a `ChildProcess`, sees no `exitCode`, and resolves without
  killing anything: ten `perf_api.py` servers (each holding a benchmark database) were still running
  before the count was checked. `await stop()` is the call; on this shared machine a leaked bench is
  RAM, and the rule is to kill only one's own PIDs, by PID.

### LN9 — Requester core: peers, decision, fetch (delivered 2026-11-18)

**What landed.** The three files that make a tap instant and a transfer honest, and nothing
else — no other file was touched:

- **`peers.rs`** — one replica of each connected peer's inventory, in memory and counted: one
  entry per track (its id, plus a bit per quality and that copy's length — 20 B), sorted so a
  lookup is a binary search. `sync_inventory` is one full pagination the first time (the
  responder answers 4 000 items a page) and one delta on every version after; one sync at a
  time per peer (an async mutex per connection, never held across the table's lock), and the
  `since` is read AFTER that mutex so a sync that finished meanwhile is not replayed. The caps
  bound what a peer can make this node KEEP, and they are checked before anything is stored:
  64 pages per sync, 50 000 tracks per peer, 32 MiB counted for every replica together, 64
  bytes per id (refused, never truncated), and a bad-track list capped at 4 096.
- **`decide.rs`** — the six-row table, pure and allocation-free. The QUALITY is decided first
  (the wanted rung if a peer has it, else the best tolerated one any peer has), then the peer
  among those that can serve it, with the note's tie-break: own device, then the peer a
  download is committed to (`prefer`), then the lowest RTT, then the device key (so two peers
  that tie on everything still answer in a stable order).
- **`fetch.rs`** — `fetch_to_file` (resume, length, SHA-256 over the WHOLE file, over-send
  detection, free space read with `statvfs` before a byte is written), `PeerStream` (the same
  request, chunk by chunk, for playback) and `probe` (symphonia: the codec the head named, and
  the duration ±2 s).

**Rules a measurement taught.**

- **A 400 KB inventory is arithmetically impossible: it is 480 KB.** The plan's "8 000 tracks
  ≈ 320 KB" assumed 40 B per item. The FINAL frame (`proto.rs`, LN0) is
  `{"track":"30000042","copies":[{"quality":"FLAC","len":16}]}` — **60 B** for a single-copy
  track — so 8 000 items are **480 223 control bytes** over two pages, counted on the wire
  (request frames and response heads, header included). What the measure is FOR still holds:
  it is paid once per peer and version, a delta is **165 bytes**, and a tap afterwards costs
  **0**. Do not re-open this: shrinking it means changing `proto.rs`, which LN0 made final.
- **`tokio::fs::File` buffers writes, so an error path must FLUSH.** The first version
  returned on a cancel or a refused over-send and dropped the handle: the last chunk (up to
  2 MiB, the file's own buffer) was lost, and a resume from "the offset we reported" landed on
  a file shorter than that offset. The transfer now flushes before it returns ANY outcome, and
  `tests/fetch.rs` asserts the disk holds exactly what the progress callback last reported.
  Every future transfer writer (LN11's downloader) inherits this.
- **`Reply::body` caps its reads at the head's `len`, which is the FULL length.** On a resume
  the body is `len - from` bytes, so that cap is `from` bytes too high — and a peer that keeps
  sending past `len` cannot be told from one that ended cleanly (both are "cap not reached").
  The transfer therefore opens the stream itself (`PeerConn::connection` + `codec`, the same
  frames) and reads exactly `len - from` bytes, then probes ONE byte: a byte there is
  `BadFile`, a clean FIN is the end. This is the only place outside `transport.rs` that
  touches `connection()`, and the reason is written above it.
- **A decision over sixteen replicas costs 11.8 µs, not 1 ms.** Measured: 1 000 decisions
  (`candidates` + `decide`) over 16 peers × 8 000 tracks, **0** requests (the `decide` call
  alone is 1.45 µs). The replica is 48 B per item, which is the 64 B budget with room;
  sixteen peers are 6.1 MB.
- **The decoder probe runs in the caller, not in the transfer.** `fetch_to_file` knows the
  length and the tag, never the duration the metadata row carries, so it guarantees length and
  SHA-256 and `probe` is a separate call the downloader makes before publishing (LN11). A
  failed probe is `BadFile`, and `Peers::mark_bad` takes the peer out of the next decision for
  that track only.
- **A cancel is `LanError::Internal("transfert interrompu")`.** No `LanError` variant means
  "the person asked", and `types.rs` is LN0's: the downloader that owns the queue (LN11)
  matches that cause and records a PAUSE, never a failure to show.

**The numbers, each with its command** (`desktop/`, `CARGO_BUILD_JOBS=4`, 2026-11-18, debug
unless said otherwise):

| Measure | Value | Command |
|---|---|---|
| an 8 000-item inventory, one full pass | **480 223** control bytes, 2 pages (60.0 B/item) | `cargo test -p nz-lan --test fetch an_eight_thousand -- --nocapture` |
| one change, as a delta | **165** control bytes, 1 request | same |
| an unchanged inventory, re-synced | **≤ 512** bytes (one empty delta) | same |
| replica memory | **48.0 B/item** — 384 096 B for 8 000 tracks, 6 145 536 B for 16 × 8 000 | same, and `memory_is_capped_for_sixteen_peers_of_eight_thousand_tracks` |
| a tap (decision) | **11.8 µs**, **0** requests, 16 replicas of 8 000 tracks | `cargo test -p nz-lan --test fetch memory_is_capped -- --nocapture` |
| `decide` alone, 16 candidates | **1.45 µs** per call over 10 000 runs | `cargo test -p nz-lan --lib decide -- --nocapture` |
| a 200 MiB fetch over loopback | **74 MB/s** debug; **450 MB/s** release | `cargo test -p nz-lan --test fetch a_two_hundred_megabyte -- --nocapture` and `--release` |
| a `Play` fetch beside a capped `Bulk` one | **4.8 ms** for 256 KiB while 1 MiB is paced at 64 KiB/s and holds the one Bulk lane | `a_bulk_fetch_holds_the_one_bulk_lane_and_a_play_never_waits` |
| the whole LN9 file | **11 tests**, 3.1 s debug (10.7 s with `--test-threads=1`) | `cargo test -p nz-lan --test fetch` |

**Mutants run once, on purpose, and the tests failed as they must** (each was reverted
immediately; `git status` was clean of them before the real commit):

- `decide`: the server check hoisted ABOVE the tolerance (rows 4 and 5 in the wrong order) →
  **5** tests fail, the 48-case table and `the_tolerance_decides_between_the_server_and_a_peer`
  included.
- the over-send probe removed → `a_peer_that_sends_more_than_its_length_is_a_bad_file` fails:
  the fetch succeeds on 4 KiB of a peer that announced 4 KiB and sent 8 KiB.
- the tag comparison dropped in `fetch_to_file` → `a_peer_that_lies_about_its_hash_is_a_bad_file`
  fails.
- `probe`'s codec check dropped → `a_file_the_decoder_refuses_is_never_published_and_the_peer_is_marked_bad`
  fails on the Opus-as-FLAC case.
- the `expect` check on a resume dropped → `a_peer_that_ignores_the_expected_tag_is_a_bad_file`
  fails: a peer that answers `FetchOk` for another file would have its bytes appended.

**What LN10 and LN11 can rely on.** `Peers::insert(conn)` on every authenticated connection
(and on a reconnect: the replica is kept, and the next `sync_inventory` asks for the delta),
`Peers::remove` on close, `Lan::decide` = `peers.candidates(track)` then `decide(…)` with
`nz_api`'s `is_online()` for `server_reachable` and `peers.complete_for(&tracks)` for
`prefer`. The opener takes `Urgency::Play`/`Prefetch` and never sees a gate; the downloader
takes `Urgency::Bulk`, the app's ONE `Gates` (`Api::gates()`), and passes its `stop` watch.

### LN10 — The `Lan` service (delivered 2026-11-18)

**What landed.** `service.rs` — the one handle the apps hold — and `tests/service.rs`, 13 tests
written from this note's *Acceptance* list rather than from the implementation: two (or five) REAL
nodes on loopback with `DiscoveryKind::External`, a fake server that signs LN1's contract with the
note's LAN seed, and fake peers that speak LN6's transport and answer LN4's `Inventory` pages.

- **Lifecycle.** `Lan::start` probes `Feature::Lan` **first**, with a throw-away current-thread
  runtime (it owns no thread): a `404` answers `Err(Unsupported)` and leaves no directory, no
  socket, no thread. Otherwise: `nz-cache`'s settings, `lan.sqlite`, the index (one `refresh`), the
  serve budget, the trust state, then the runtime — one worker, threads named `nz-lan`, **niced +5**
  (`libc::setpriority`).
- **The driver** is one `tokio::select!` over the app's commands, its children's reports, the
  activity flags, the discovery feed, the index's version and four timers. Nothing awaited inside it
  can block it: enrolment/renewal is a child task, the accept loop is a child task (a handshake must
  never be cancelled by another event), and each authenticated peer gets two children — a serve loop
  and a control loop.
- **Enrolment and renewal.** The cached credential first (`load_cached`, **0 requests**: the trip
  works with the server unreachable), then — only when a credential is already held — one
  **conditional** roster sync (`ETag`, `304`, 0 bytes), then `ensure_enrolled`. A renewal rotates
  the key and the endpoint's identity **before** the snapshot says so (`set_identity`), so no screen
  and no peer ever sees a credential this node does not serve yet. `Unsupported`/`Disabled` stop the
  node and say which; any other failure keeps the cached identity, fills `key_error` in French, and
  retries on the next daily tick or on `enroll_now`.
- **Discovery, dialling, gossip, replication.** The feed is created once and started/stopped with
  the activity; a `Found` dials (bounded table, one dial per instance per [`MIN_REDIAL`], ten
  minutes after a refusal); an authenticated connection is admitted once per device, remembers its
  address (`peer_addr`, the reconnect path), offers the trust this node holds, announces the
  inventory version and then replicates the peer's. A peer whose inventory moves is re-synced on its
  `InventoryChanged`.
- **Serving** goes through LN4's `Server::handle`, so the serve budget and the `Bulk` lane are the
  same mechanism. With « Partager mes sons » off the node answers the frames of a node that **has
  nothing** — an empty `InventoryFull`/`Sets`, `Meta` with no rows, `NotFound` for a fetch — never a
  refusal that would tell a friend more than an absent file.
- **Battery.** `set_active` feeds three counted flags; when none is on, `SLEEP_GRACE` (60 s) later
  the connections close with code 0 (never `CLOSE_REFUSED`: a phone whose screen went off must not
  cost a peer ten minutes of silence) and the discovery stops. Nothing is dialled, announced or
  synced afterwards; `enroll_now` still works (an HTTP round trip is not a LAN packet).
- **`enabled = false`** closes the socket too (the port is free again) and stops the hashing
  worker; turning it back on re-reads the cached credential, binds, announces and dials the known
  addresses.
- **`shutdown()`** stops the hashing worker (its own thread and runtime), closes the endpoint (which
  closes every connection and joins quinn's only task), stops the discovery (which joins its
  daemon), aborts and drains its children, and joins the runtime's thread. It is idempotent, and a
  `Node` in a test shuts down on `Drop` so a failing test cannot leak threads into the census.
- **The snapshot** is one short mutex plus the counted replicas: `state` (derived: `Connected(n)`
  from the peer list), `key_until`, `key_error`, the peers with name/relation/platform/RTT/tracks,
  the transfers in flight, the last French failures, the port and its fallback.
- **Rule 1** everywhere: the enrolment (`Prefetch`/`Bulk`), the LAN inventory sync (`Bulk`, the SAME
  `Gates` as a download) and the responder's `Bulk` serves all yield to what the person just did;
  serving, hashing and the runtime are niced (+5, +10).

**Additions to LN0's surface** (each additive, each with its reason):

1. **`Lan::wire() -> WireCounts`** (`packets_out`, `packets_in`, `bytes_out`, `bytes_in`, `dials`)
   — quinn's OWN per-connection counters, sampled on the runtime (never in `snapshot()`:
   `Connection::stats` locks the connection). It is the battery measure the note asks for ("0 LAN
   packets": a counter, not `tcpdump`), and a Réglages line may print it.
2. **`Lan::set_sleep_grace(Duration)`** — a test cannot pause the clock of a runtime it does not
   own, so the 60 s of grace would make every run pay a minute. The app keeps `SLEEP_GRACE`.
3. **`Lan::settings()` answers the EFFECTIVE settings**: `LanConfig.port` overrides the stored one
   for the session (`0` = any, which is what a test needs), without rewriting the person's setting.
4. **`Lan::start` inside a runtime skips the probe** (`has_route` is async and blocking there
   would panic): the driver asks as its first act, and a `404` then answers
   `LanState::Unsupported` and stops everything. Both halves are tested.

**Where a divergence is deliberate:**

- **The gossip offers with an UNKNOWN peer** (`serial 0`, no roster serial): `PeerInfo` does not
  carry the peer's revocation serial (LN6's shape, and this lot does not own that file). The cost is
  at most one revocation list and one roster per connection — the receiver keeps the maximum and
  ignores what is not newer (`gossip::accept` answers `Ok(false)`) — and it is bounded by the
  connection's own lifetime, not by a timer.
- **`transfers` holds the serves this node answers** (track, peer, urgency, the bytes written when
  it ends); LN11's downloader registers its own with a total. The registry is capped at 32 rows so a
  peer's 64 streams cannot make this device keep 64.
- **`sets()` answers `Vec::new()`**: the sets replica is `sets.rs`'s (LN11). The peer list's `sets`
  column reads `Peers::views()`, which is 0 until then — one source of truth, not two.

**The numbers, each with its command** (`desktop/`, `CARGO_BUILD_JOBS=4`, debug unless said
otherwise, 2026-11-18):

| Measure | Value | Command / test |
|---|---|---|
| the lot's file | **13 tests**, 45.5 s | `cargo test -p nz-lan --test service` |
| the crate after it | **101 unit + 19 cred + 5 discovery + 23 enroll + 11 fetch + 19 serve + 13 service + 20 transport**, all green | `cargo test -p nz-lan` |
| two nodes, first login | **2 status + 2 enrol + 0 roster** requests; **1** dial for the pair; `Hello` names both; B replicates A's 2 tracks | `two_nodes_enrol_find_each_other_and_replicate_their_inventories` |
| the pair's traffic | **8–10 datagrams out, 6 in, 5 834 B out**, RTT **0–1 ms** | same (`Lan::wire`) |
| asleep | A **9 datagrams / 5 895 B**, B **15 / 8 708** — and **0 new** after `set_active(all false)`, across a `found`, a `network_changed` and an `enroll_now` | `a_paused_node_closes_everything_and_sends_nothing` |
| the connections close | **at once** (asserted with a **5 s** net, well under quinn's 30 s idle timeout) | same |
| `enabled = false` | state `Off`, peers 0, the UDP port is **bindable again**, `wire()` frozen; back on → **1 dial** of the stored address → 1 peer | `switching_the_feature_off_stops_everything` |
| a server without the feature | `Err(Unsupported)`, **1** request (the `OPTIONS` probe), **no** directory, **0** thread | `a_server_without_the_feature_starts_nothing` |
| a server that loses it | `LanState::Unsupported`, peers 0, `wire()` frozen | `a_server_that_loses_the_feature_stops_the_node` |
| a renewal | key rotated, device row kept (**2** rows for 3 enrolments), B still on the OLD key (**1** peer), **the live connection still serves** (B gains a track, A's replica grows), and a third device authenticates with the NEW key | `a_renewal_swaps_the_identity_and_keeps_the_live_connection` |
| gossip | A learns serial 8 from the server, B's `lan.sqlite` ends with **serial 8 + the tablet's key**, and BOTH refuse the tablet (C's own peer count stays 0) | `a_revoked_device_is_refused_by_both_peers_through_gossip` |
| the ten-minute silence | 3 repeats of both addresses → **0** extra dial (a never-seen address queued behind them is dialled: the marker) | same |
| « Partager mes sons » off | an `Inventory` answers **0 items, version 0**, a `Fetch` answers `NotFound`, and no request fails | `a_device_that_shares_nothing_answers_an_empty_inventory` |
| shutdown | **0** `nz-lan` and **0** `nz-lan-hash` threads after it, twice called, and the node still answers `snapshot`/`decide` | `shutdown_joins_every_thread` |
| memory, 4 peers × 8 000 tracks | **1 537 949 B counted = 48.1 B/item** (the replica is 48 B/item; the service's own counters add < 64 KB) | `four_peer_inventories_fit_the_memory_cap` |
| memory over 10 000 deltas | **384 841 → 384 841 B (0.00 %)**, 10 000 deltas APPLIED in 6.1 s | `the_memory_is_flat_over_ten_thousand_inventory_deltas` |
| the snapshot | **482–974 ns** per call with a peer connected; `decide` **1.18–1.57 µs**; **0** requests for 1 000 taps | `the_snapshot_and_a_tap_never_touch_sqlite_or_the_network` |
| rule 1 | the person **0.79–1.19 ms**; the node's background request after **1.0018 s**; a peer's first inventory after **1.0019 s** (the SAME `BULK_QUIET`) | `a_bulk_lan_request_yields_to_the_persons_request` |
| lints, Android shape | `clippy -D warnings --all-targets` clean with and without `mdns`; `mobile/rust` `cargo check --workspace --locked` clean; **neither `Cargo.lock` moved** | the CI commands |

**10 mutants, one per property, each run once and each RED** (`target/ln10-mutants/mutants.py`,
restored and `touch`ed after each — LN2's lesson): « Partager mes sons » ignored → the empty-frame
test; no gossip on connect → B's store stays serial 7; the dial table always says yes → the
backoff unit test; a renewal that keeps the old identity → the third device sees the old key;
falling asleep that keeps the connections → the peers-gone assertion; a server without the feature
that starts anyway → the `Err(Unsupported)` test; `shutdown` that leaks the hasher
(`mem::forget`) → the thread census; the LAN sync without the `Bulk` lane → the rule 1 timing;
`memory()` without the replicas → the memory cap; `enabled = false` that keeps the socket → the
bind-the-port assertion.

**Rules a measurement taught.**

- **`publish` only wakes when the VIEW changed, so everything outside it needs `touch()`.** The peer
  list, a replica, a renewal that lands on the same second and a refused connection all live outside
  the view's fields: the first version published `|_| {}` (a no-op) and a test waited its **whole
  40 s deadline** while the state was already correct, and a screen would have kept a device that
  had gone. `touch()` is the version bump without a view change.
- **`fail()` keeps ONE line per cause, so a repeated failure is not an event.** The second refusal
  of the same peer changes nothing in the view: an attempted connection therefore `touch()`es on its
  own (it is what a peer list redraws for).
- **A `Bulk` lane held at CONNECTION scope starves every other peer.** Taken once before the control
  loop's `while`, the guard lived for the connection's life and 2 of 4 replicas never arrived
  (measured: 40 s, `tracks: 0`). The lane is taken around the `sync_inventory` await and nowhere
  else.
- **A re-dial after a close is a handshake loop unless it is fenced.** With the duplicate rule
  collapsing a pair, the loser's close triggered a dial, whose duplicate close triggered another:
  RTT grew to 1.2 s, no inventory ever finished, and `memory()` fell to its floor. Fixed by dialling
  only when the DEVICE has no connection left, and by a per-instance floor (`MIN_REDIAL`, 2 s)
  beside the ten-minute backoff.
- **A renewal must not re-dial.** Doing so gives the peer a second device row — the rotated key IS
  a new identity — so B saw 2 peers where the note promises 1. The endpoint swaps its identity;
  nothing re-connects.
- **A connection closed by the sleep arrives AFTER the map was drained.** `Closed` was matched
  against `conns`, the sleep had already emptied it, and the peer's replica stayed for ever. The
  message removes the replica even when the connection is no longer in the map.
- **quinn's idle timeout can pass for work the node did.** A mutant that removed the close on sleep
  still passed a 40 s bound: the connection died at 30 s by itself. The bound for "it happened
  because the node did it" is 5 s — far under `IDLE_TIMEOUT` — and the same reasoning applies to
  the "closed the socket" assertion (bind the port).
- **Real packet counts are quinn's, and they belong off the UI thread.** `Connection::stats()` gives
  `udp_tx`/`udp_rx` datagrams and bytes per connection; sampling it in `snapshot()` would lock the
  connection on the UI thread, so the driver samples it (on admit, on close and on a 1 s tick while a
  peer exists) into atomics.
- **A test that counts threads must own the process.** `/proc/self/task` is process-wide: 13 tests
  in one binary would count each other's runtimes. One mutex serialises the file (and a `Drop` on
  the test's `Node` shuts the node down when a test panics, or the failed test's threads poison the
  census of the next one).
- **The 60 s of grace cannot be paused from outside.** `tokio::time::pause` affects the runtime it is
  set on, and the node owns its own: hence `set_sleep_grace`, and the *unit* test that spells
  `SLEEP_GRACE == 60 s` (LN7's rule: a test must not take its boundary from the constant it tests).
- **A `/proc` scan must skip the entry it cannot read.** `thread_by_name` returned `None` for the
  WHOLE scan when one entry vanished between `read_dir` and the read of its `comm`, and under 101
  parallel tests that is a thread of another test exiting in exactly that window: measured **2
  failures in 8 runs on `master`**, 0/3 in series, 0/5 alone. The lookup now skips what it cannot
  read and waits — bounded, 10 s — for the name to appear (a `spawn` returns before the child has
  set it), so a thread that never takes its name still fails, with the name in the message. Fixed in
  this lot's PR because LN4 was delivered and nobody else writes `hashing.rs`: **0 failures in 50
  parallel runs** after, and the same test fails in 10.0 s when the worker's name is changed.
- **A fake server's credential window must move for a renewal to be observable.** `key_until` is the
  only screen-shaped trace of a new credential, and a renewal that lands in the same second leaves
  it unchanged: the fake issues `at + days + n` for the n-th credential of a device, which is what a
  real `now + 30 days` is one second later.

### LN11 — Adapters: opener, downloader, sets, covers and metadata (delivered 2026-11-18)

**What landed.** The four seams the apps call, filled, plus the one thing they needed from the
service: `PeerFirstOpener`, `PeerFirstDownloader`, `sets.rs`'s replica and `download_peer_sets`, and
`import.rs`'s metadata and sleeves. Nothing public in `nz-lan` changed shape; `service.rs` gained a
`pub(crate)` block (see *Corrections*), declared out of the lot's file list in the PR.

- **`opener.rs`** — `open` decides (`Lan::decide`), and for a peer opens a `Fetch` at the lane's
  urgency (`User → Play`, the rest 1:1) and returns a blocking body whose `len_hint` is what remains
  of the file and whose `quality()` is the rung the peer really served (LN5). `start` never goes to a
  peer; a seek re-opens at `from_byte`. The choice is sticky per track (256 entries): a byte-range
  reopen asks the peer that served the stream, at the quality it served — even when a fresh decision
  would have picked another device. `ready()` answers `ready = true, bytes = len, ranges = "bytes"`
  from the replica, **0 requests**. No connected peer → the inner opener, 0 ms added.
- **`downloader.rs`** — `fetch` runs `Lan::decide_download` (`Bulk`, with the set's committed peer as
  `prefer`), and for a peer writes the worker's `.part` through LN9's `fetch_to_file` (length,
  SHA-256, free space, over-send), probes it with the decoder, and answers `Fetched { bytes, quality:
  the peer's }`. The transfer runs on the `nz-lan` runtime and progress crosses back through a
  channel, because `nz_offline::Progress` is **not `Send`** and may only be touched by the worker's
  thread. A `NotFound`/`Busy`/bad file/dead stream skips the peer and the inner fetcher takes over;
  a `BadFile` also `mark_bad`s it so the person's retry goes to the server.
- **`sets.rs`** — `Replica` holds each peer's `Sets` pages, counted and capped (512 sets/peer, 20 000
  tracks/set, 8 MiB for all peers); the control loop replicates it on every connection and on every
  `SetsChanged`, on the **Bulk** lane, so `Lan::sets()` is a short lock with **0 requests**.
  `download_peer_sets` re-checks completeness against that live replica (one `Sets` pass only when it
  has never heard the peer), takes the labels and sleeves from the same peer, records the peer as the
  tracks' commitment, and queues ONE `nz-offline` job.
- **`import.rs`** — `Meta` 200 ids a frame, one `Cover` per missing sleeve, capped before any
  allocation (2 MiB a sleeve, 64 MiB a set), stored PINNED, and the titles/artists handed back for
  the `Choice` labels. A sleeve is named by the digest of its URL, never by the URL: nothing the wire
  says becomes a path.

**What a later lot can rely on.**

- `PeerFirstOpener::new(inner, lan)` / `PeerFirstDownloader::new(inner, lan)` wrap the app's own
  seams and change nothing else; `Lan::sets()` and `Lan::best_peer_for()` are what « Sur vos
  appareils » and the playlist header read.
- `Lan::decide_download(track, quality)` is the same table as `Lan::decide` with the set's committed
  peer as its `prefer`.
- The four adapters go through `import::run_on`, which never panics: a plain thread `block_on`s the
  node's runtime, a thread INSIDE another runtime spawns the future and waits on a channel, and a
  caller standing on the `nz-lan` runtime itself is told `None` so it can take the server.

**The numbers, each with its command** (`desktop/`, `CARGO_BUILD_JOBS=2`, debug, 2026-11-18):

| Measure | Value | Command / test |
|---|---|---|
| tap → first audible frame from a peer | **4.3–10.9 ms** (loopback) | `cargo test -p nz-lan --test integration a_track_only_the_peer_has -- --nocapture` |
| a track no peer has | **1.11 ms** from `load` to the inner opener's first byte, **1** open, **0** LAN datagrams | `... a_track_no_peer_has -- --nocapture` |
| a decision over a replica | **1.32 µs** (1 000 calls = 1.32 ms, under the 100 ms a single request's worth would need), **0** request | same |
| `ready()` | **7.0 µs** per call (10 000 = 69.8 ms), so **0** requests (one round trip would be ≥ 3 s) | `... a_seek_reopens_the_same_peer -- --nocapture` |
| a peer that stalls mid-track | takeover at **exactly 65 536 B** (one 64 KiB chunk), the spool byte-identical to the file | `... a_stalled_peer_hands -- --nocapture` |
| a 50-track playlist from one peer | **50/50** files, **0** opens of the server, labels and pinned sleeve present; **35.5 s** through the worker (the Bulk gate) | `... a_fifty_track_playlist -- --nocapture` |
| the same, server unreachable | **50/50** from the peer, list from its `Sets`, **0** server opens | `... the_same_playlist_downloads -- --nocapture` |
| a set replica, with its inventory | **3 924** counted bytes for 50 tracks (**78 B/track**: 1 524 for the sets, 2 400 for the inventory) | playlist test |
| rule 1 | first sample **7.7 ms idle → 6.7 ms** while downloading from a peer AND serving another; **0** underruns | `... playing_while_downloading -- --nocapture` |
| the lot's file | **8 tests**, ~154 s | `cargo test -p nz-lan --test integration` |
| the crate | **108 unit + 19 cred + 5 discovery + 23 enroll + 11 fetch + 19 serve + 13 service + 20 transport**, green | `cargo test -p nz-lan` |
| the crate for Android | `cargo check --workspace --locked` green in `mobile/rust`, neither lock moved | the `mobile` workflow's command |

**Rules a measurement taught.**

- **A splice is only honest between the SAME bytes.** The note's "the server takes over from the byte
  already received" cannot hold when the peer serves a tolerated rung other than the one asked, nor
  for an Opus transcode (not byte-reproducible): half of each is a file no decoder reads. `take_over`
  therefore splices only when both sides are FLAC — the archive, byte-identical everywhere — and
  otherwise stops the body and `mark_bad`s the peer, so the app's « Réessayer » fetches a clean file
  from the server. Proved: a stall at 65 536 B gives `opens == [Some(65536)]` and a spool equal to
  the original (`a_stalled_peer_hands...`); the same stall on an Opus 320 request never reaches the
  server at an offset.
- **`Progress` is not `Send`, so a peer download cannot publish progress through a closure.** Passing
  `&mut dyn Progress` into `fetch_to_file`'s callback cannot compile, and spawning a thread for it
  would not either. The transfer runs on the node's runtime and reports `(bytes, total)` over a
  channel the worker's thread drains: live bar, no `unsafe`, no second thread.
- **A counter sampled once a second is not an event.** `Lan::wire()` is read by the driver on a 1 s
  tick, so "the peer served it" asserted on it fails for a transfer shorter than that — and a 3 s
  playback makes the READING grow by itself (measured in CI: 9 → 21 packets for a track nobody
  has). The tests prove it with what cannot lie: the inner opener's **open count**, the quality the
  event carries, and the spool compared byte for byte with the peer's file; the zero-request
  property is asserted on the COST of 1 000 decisions (1.3 ms — a single loopback request is
  0.3–1 ms, so 1 000 would be ≥ 300 ms), never on the sampled counters.
- **Stickiness is only observable where a fresh decision would differ.** Two peers with the same
  rung would make the assertion pass without the sticky map. The test gives A the FLAC and C the
  Opus 128, opens at FLAC (A), then seeks asking Opus 128 — where `decide` answers C — and asserts the
  bytes are A's FLAC slice.
- **A `Sets` answer is paginated and versioned, but a replica is re-read whole.** 64 sets a page and
  a set that moves rarely do not justify a delta protocol `proto.rs` does not have; the version is
  logged, not kept.
- **The connect burst is ONE `Bulk` window, sets first.** Two `acquire`s would make the second sync
  wait the quiet window again (a peer's files twice as long to become usable), and inventory first
  let LN10's own measurement — 1 000 snapshots and taps with "0 request" counted on the peer — start
  in a window where the sets request had not landed yet: measured in CI, **3 requests instead of 2**,
  and `the_snapshot_and_a_tap_never_touch_sqlite_or_the_network` went red without a line changed in
  it. The sets are replicated first so that a caller which waits for the inventory already has them;
  the LN10 suite is green untouched, three runs in a row.
- **`nz-offline`'s worker used to stop entirely when `Api::is_online()` was false**, so a
  peer-served playlist on a trip could not be QUEUED while the app knew it was offline. Fixed in the
  addendum below: the gate now asks the fetcher, and a peer with files is a source.

**Corrections to this plan, each proved by the code or a measurement:**

1. **`LanEvent::Served` does not exist.** LN0 made `Lan`'s surface final and no such event is in it;
   inventing one here would be a second source of truth. The opener reports the served quality through
   LN5's `ByteStream::quality()`, the decision is `Lan::decide`, and the tests prove the source with
   the inner opener's open count and the spool's bytes.
2. **`PeerFirstOpener::new` needs `O: 'static`.** A body that falls back mid-track must hold the inner
   opener, and `Box<dyn ByteStream>` cannot borrow it. The wrapper holds `Arc<dyn StreamOpener>`;
   `O` stays the contract's type parameter (marker), and an opener lives for the process anyway.
3. **`service.rs` gained a `pub(crate)` block** — `handle()`, `conn(&DevicePub)`, `api()`, `cache()`,
   `peer_len(track, quality)`, `mark_bad(peer, track)`, `peer_set(peer, kind, ident)`,
   `commit_peer_tracks(peer, tracks)`, `decide_download(track, quality)` — and `sets()` now answers
   the replica. `Shared` gained two fields (`sets: Replica`, `handle: Mutex<Option<Handle>>`) and the
   control loop replicates the sets. Without the doors the adapters could not open a transfer at all
   (`Shared` is private to its module, and a sibling module cannot see `self.inner`); no existing
   behaviour changed and `tests/service.rs` stayed green untouched. Approved by the orchestrator
   before the edit.
4. **The playlist's labels live in `nz-offline`'s kv, not in its snapshot.** `Snapshot::labels` only
   carries `kind == "track"` rows, so a playlist's titles are asserted on `LABELS_KEY` — which is also
   where the peer's `Meta` answer reads them on the next import.

#### Addendum (2026-11-18): the downloads worker's source

**What landed.** `nz_offline::Downloader::reachable() -> bool`, with a default of `true` and two
overrides: `ApiDownloader` answers the server's own `is_online()` (exactly what the worker asked
itself before), and `PeerFirstDownloader` answers « the server is reachable **or** a connected peer
holds at least one file ». `service::run`'s gate asks the FETCHER instead of the `Api`, so a queue
runs on a trip without data and the rows nobody has fall back to the server's own `Offline` outcome —
a « later », never a verdict (rule 3, unchanged).

**What a later lot can rely on.** An `ApiDownloader` behaves exactly as before (its own test
`a_dead_link_keeps_the_work_queued` stays green untouched); a custom platform fetcher keeps the
default and is not asked to know about the network; a LAN fetcher says when a peer can serve.

**Measured** (`desktop/`, debug, 2026-11-18): a playlist of **50** tracks queued with the server
unreachable and one peer holding it complete → **50/50 published**, the inner fetcher called **0**
times, the worker **never** `Offline`
(`cargo test -p nz-lan --test integration the_same_playlist_downloads -- --nocapture`); with no server
AND no peer → state `Offline`, **0** download, **0** call (`… a_worker_with_no_source_stays_offline`);
the fetcher's own verdict is what decides, whatever the link says
(`cargo test -p nz-offline --lib a_fetcher`).

**Rule.** The worker's gate asks the thing that KNOWS. `Api::is_online()` was a good enough proxy
while the server was the only source; the moment a peer can serve, asking the link becomes the bug —
it pauses the queue for a source that is up.

### LN12 — Desktop wiring and screens (delivered 2026-11-18)

**What landed.** The desktop app is a node, and the two screens that make it one.

- **`wiring/lan.rs`** — the process's [`Lan`]: `attach` (blocking; the probe, `lan.sqlite` and the
  index read), `start_in_background` (the ONE call site, `wiring::engine::spawn_audio`, i.e.
  `AppState::start_audio` at a login or a restore), `save_settings` (persist in `nz-cache`'s `kv`
  **and** apply to the running node, so a click that lands while the node is still starting is not
  forgotten), `snapshot`/`version` (0 requests), `peer_sets`/`download_sets`, `best_peer_name`,
  `key_line`, `french_date`, `firewall_line`, and the two wrappers: [`LanOpener`] and
  [`LanDownloader`].
- **The wrappers are transparent until the node exists, then peer-first.** `LanOpener` asks
  [`current`] on each `open`/`ready` and builds LN11's `PeerFirstOpener` once per node generation;
  before that every method is the inner opener's (**0 ms added**, the note's « No peer has it »
  row). `LanDownloader` does the same around `ApiDownloader`, and **delegates `reachable()`** — the
  gate #352 added to `Downloader`: without a node the answer is the server's own link, with a node
  it is « the server OR a connected peer ». This is what makes the login instant: the probe and the
  SQLite read happen on a `nz-lan-start` thread, and neither the engine nor the worker waits for it.
- **`screens/settings/lan.rs`** — Réglages → « Appareils à proximité », its own screen like
  Téléchargements (seven cards: cet appareil + la ligne `ufw`, les deux interrupteurs, les quatre
  qualités, les appareils connectés avec leurs échanges, mes appareils, les amis de confiance,
  avancé). One timer (500 ms) compares `lan::version()` and re-reads the snapshot only when it
  moved; the three server lists (`/lan/devices`, `/lan/friends`, `/me/playlists`) are asked ONCE per
  open on `nz-net`, through the screen's own channel.
- **`screens/peer_sets.rs`** — « Sur vos appareils », mounted in Réglages → Téléchargements (one
  `.child` there): each peer's sets with track count, size and « complet », multi-select, and
  `nz_lan::sets::download_peer_sets` on « Télécharger ». It draws **nothing** when this app is not a
  node.
- **The playlist header** and **En lecture** say where the sound comes from: `Lan::best_peer_for`
  once per page load (and per node change), and « Lu depuis PC (FLAC) » from the ring below.
- **The quit joins the node LAST** (`main.rs`'s `NetRuntime::drop`): offline, engine, then `Lan`.

**Additions to the plan's surface** (each additive, each with its reason):

1. **A served-source ring, `wiring/lan::record_served` + `source_line`.** LN11's correction says
   there is no `LanEvent::Served`; the opener wrapper therefore records `(track → peer name,
   quality)` **at each successful peer `open`**, reading the name from the snapshot then (a peer may
   disconnect while its track plays). Bounded to 64 rows (`SERVED_LIMIT`), oldest out first.
2. **`Lan::port` is passed through, `0` included.** LN10's `Lan::start` overrides the stored port
   with `LanConfig.port`; the app passes the device's own setting so « 0 = any » (and a test's
   ephemeral port) is honoured rather than silently replaced by 45910.
3. **Three files the plan's LN12 row did not list, approved by the orchestrator before the edit and
   declared in the PR**: `desktop/app/Cargo.toml` (the `nz-lan` dependency — no crate the app
   already depended on re-exports it), `desktop/app/src/nav.rs` (`SettingsSection::Lan`, its slug,
   its title, and its place in `USER` — where the SPA puts « Appareils et amis »: after Sources,
   before Compte), and `desktop/app/src/main.rs` (one call in `NetRuntime::drop`, plus the three
   glyphs `Laptop`, `Radar`, `Smartphone` in `SHELL_ICONS` **and** the `icon_assets!` list — the two
   lists are separate, and a glyph in one only renders nothing).

**Where a divergence from the plan is deliberate:**

- **The section is a `delegated` screen, so the category strip is not drawn while it is open** —
  exactly like Téléchargements, Utilisateurs and Serveur (D14a's shape). The breadcrumb above stays
  (« Réglages · Appareils à proximité », with the back arrow), and the tab strip is one row again on
  the other sections.
- **The `ufw` line lives in the first card, beside the port it opens**, not in « Avancé ». It is the
  one thing the person must run on this machine, and at 1 280×800 « Avancé » is below the fold: a
  screenshot found it, not a test (the note's rule 5: what the person cannot see is not there).
- **A device's `last_seen` is printed as a day** (« vu le 18 nov. 2026 ») through a small parser; the
  server sends an ISO instant and printing it raw was the first version of the row.

**The numbers, each with its command** (`desktop/`, `CARGO_BUILD_JOBS=2`, debug, 2026-11-18):

| Measure | Value | Command / test |
|---|---|---|
| the app's tests | **406 passed, 8 ignored, 0 failed**; **18** are this lot's | `cargo test -p neezer-desktop --bins` |
| an older server (no `/api/lan/status`) | `attach` answers `None`, **0** `nz-lan` threads, **no** `$data/neezer/lan` directory, and a revisit costs **0** extra request (the refusal is remembered) | `wiring::lan::tests::a_server_without_the_feature_starts_nothing_at_all` |
| the section on an older server | the strip has **no** `tab-Lan`; the route answers « L'échange entre appareils proches » instead of painting | `screens::settings::ui::the_lan_section_is_its_screen_and_hidden_without_the_feature` |
| a LAN-enabled server | the node runs (**≥ 1** `nz-lan` thread), binds a port, and an enrolment the server refuses puts its French cause in `key_error` | `wiring::lan::tests::a_lan_server_gets_a_node_that_says_where_its_key_stands` |
| enrolment + « Clé valable jusqu'au 18 décembre 2026 » | **end to end**: a fake server signing LN1's credential with the note's LAN seed, the app under Xvfb inside `unshare -rn`, state « Enregistré — recherche des appareils » | the capture `docs/agents/screenshots/lan/LN12/appareils-a-proximite-2560x1600.png` (no test: a signing fake is LN10's, in `nz-lan/tests/service.rs`) |
| the key line | « Clé valable jusqu'au 15 janvier 2027 » for `1 800 000 000`, and the CAUSE when a renewal failed | `the_key_line_is_a_date_or_the_cause_never_an_empty_state` |
| the `ufw` line | `sudo ufw allow 45910/udp`, and `… 38462/udp` when the node fell back | `the_firewall_line_names_the_port_really_bound` |
| the source line | « Lu depuis PC (FLAC) », « Lu depuis Téléphone (Opus 320) »; the ring holds **64** rows and forgets older ones | `the_source_line_names_the_peer_and_the_quality` |
| the ring's memory | **5 248 B** for 64 rows (**82 B** a row: 56 B of row + 7 B of id + 19 B of name); the wiring counts **0** with no node and an empty ring | `the_wiring_memory_is_the_node_plus_one_bounded_ring` |
| no node ⇒ the wrappers cost nothing | one `open` = one inner `open`, nothing recorded, `ready` unchanged | `without_a_node_the_opener_is_the_inner_one` |
| the quit | the node is **still running** after `offline::shutdown_process` + `engine::shutdown_process`, and **0** `nz-lan` threads when `lan::shutdown_process` returns; idempotent | `the_quit_joins_the_node_after_the_worker_and_the_engine` |
| the section's layout | three windows (2 560×1 600, 1 280×800, 660×800 = the 420 px panel's column): the column never overflows, and the two switches, the four quality boxes, `lan-copy-ufw` and `lan-renew` all have a box | `screens::settings::lan::ui::the_section_fits_the_three_windows` |
| a click acts | one click on « Partager mes sons » writes `lan.serve = false`; one on Opus 64 adds `OPUS_64`; turning the last rung off is refused and the set keeps one | `…::a_click_writes_the_device_settings_and_forgets_nothing` |
| the settings strip | **11** categories in **one** 33 px row at 1 280 px (the Lecture et son pane's last control then sits at y = 781.5 in an 800 px window) | `…::the_mouse_switches_sections_and_flips_a_setting` |
| « Sur vos appareils » | not drawn at all without a node; a 4 000-track set keeps **no** track id in the row, and its key is the `"<peer hex>:<kind>:<ident>"` triple (never a path) | `screens::peer_sets::tests::*` |
| the screen-build bench (D7's guard) | **1 837 456 B** for 2 000 builds (budget 2 097 152) | `cargo test -p neezer-desktop --bins -- app::mouse::building_a_screen --nocapture` |
| lints, format | `clippy --all-targets -- -D warnings` and `cargo fmt --check` clean | the CI commands |
| the locks | **neither** `Cargo.lock` moved (`nz-lan` was already a workspace member) | `git status desktop/Cargo.lock mobile/rust/Cargo.lock` |

**Rules a measurement taught.**

- **A per-screen allocation is a leak the shell's own bench catches, not this lot's tests.**
  `wiring::offline::attach` re-wrapped the fetcher on every call, and it is called from every screen
  that watches the worker: D7's `building_a_screen_a_thousand_times_keeps_nothing` read **3 469 456 B**
  for 2 000 builds against a **2 097 152 B** budget (baseline **1 822 327 B**). Two more findings came
  out of the same bisect: `Offline::start(store)` does **not** publish the process singleton
  (`Offline::ensure`/`get` do), so a hand-rolled "ensure" created a whole worker per call; and a
  wrapper must be installed **once per session** (`install_downloader`, comparing server and owner
  without allocating) because `ensure` re-installs the plain fetcher on every call.
- **An eleventh settings category takes a row, and that row is 32 px the panes do not have.** At
  1 280×800 the Lecture et son pane ends at y = 780 with ten tabs; the wrapped strip pushed its
  « Mono » switch to y = 812 — **outside the window**, so a real click did nothing. The chips are
  drawn at `rems(0.6875)` with `px(rems(0.4375))` now, the strip is **33 px** (one row), and the
  switch is back at 781.5. A layout regression of one row is invisible in a screenshot of Réglages
  and fatal to a control at the bottom of a pane.
- **`debug_bounds` only sees `debug_selector`, not `.id()`.** The delegated area had an `id` and no
  selector, so "the section is its own screen" could not be asserted until the selector was added.
- **`LanSnapshot::default()` is `Off`, and that is the wrong sentence for "no node yet".** A device
  whose node has not started (or a build whose server has no feature) must read « pas encore
  enregistré », never « désactivé » — the screen builds `NotEnrolled` explicitly when
  `lan::snapshot()` is `None`.
- **The port a snapshot names before an endpoint exists is the SETTING, not a bound socket.** With
  `lan.port = 0` (a test's ephemeral port) an unenrolled node reports `0`; the card says « choisi par
  le système » and the firewall line then follows the port the node really bound once it has one.
- **A fake server's `/api/lan/enroll` answering `404` means "no such feature" to the node** (it stops
  and forgets the screen), so a test of a *failed renewal* must answer `5xx`: the cause then lands in
  `key_error`, which is what « Réglages » prints.
- **The `icon_assets!` list and `SHELL_ICONS` are two lists.** A glyph added to one and not the other
  compiles, renders **nothing**, and fails `every_icon_name_used_in_the_app_is_in_the_bundle` — the
  check that exists because an invisible button keeps its hit box.

**What a later lot can rely on.**

- `wiring::lan::{attach, start_in_background, current, snapshot, version, save_settings,
  peer_sets, download_sets, best_peer_name, peer_named, source_line, record_served, key_line,
  french_date, firewall_line, memory, shutdown_process}`; `install_downloader` is called by
  `wiring::offline::attach` and must stay the only installer.
- The ring and the two wrappers are bounded and transparent without a node, so a screen or a test can
  call `source_line`/`peer_named` unconditionally.
- **The offline gate is #352's, merged during this lot**: `Downloader::reachable()` replaced the
  `Api::is_online()` check in `service::run`, so a queue runs on a trip when a peer holds the files.
  This lot only has to *answer* it (`LanDownloader::reachable`), and « Sur vos appareils » keeps one
  sentence for the case that still exists — the worker `Offline` means neither the server nor any
  connected peer has the tracks (`the_download_fetcher_answers_the_worker_s_reachability_question`).

### LN13 — Mobile core and FFI (wave 6, effort high)

- **Objective**: the phone's Rust core is a node: `nz-core::lan` owns `Lan` (`DiscoveryKind::
  External`), wraps the player's opener and the offline downloader, and `nz-ffi::LanApi` exposes it
  to Kotlin with a change-only listener.
- **Files**: LN13 row. **Contracts**: *Mobile FFI* in *Contracts*; mobile.md's FFI rules (one impl
  block per file, screen-shaped records, events on change only, `NzError` with a French sentence).
- **Depends on**: LN11; mobile MB3b and MB15 merged (the opener and the downloader it wraps).
- **Acceptance**: host `cargo test --workspace --locked` in `mobile/rust`: two cores (temp dirs under
  `target/`) find each other through `found`, the second plays a track only the first has; `LanApi`
  methods return within one frame for the snapshot (atomic/clone read), events fire on change only
  (a 10 min idle test with the paused clock: 0 events); `set_active(false, false, false)` → no
  packet; `shutdown` joins `Lan`; `cargo ndk -t arm64-v8a -P 26 build -p nz-ffi` green and
  `libnz_ffi.so` grows by ≤ 1.5 MB (arm64, release — report the number).
- **Prompt**:

  > Tu implémentes le lot **LN13 — Mobile core and FFI** du plan « nœuds LAN » (effort high). But :
  > le noyau Rust du téléphone devient un nœud — `nz-core/src/lan/` possède `Lan` (découverte
  > externe, alimentée par Kotlin), enveloppe l'opener du lecteur et le downloader hors ligne, et
  > `nz-ffi/src/lan.rs` expose `LanApi` (instantané en lecture immédiate, écouteur qui ne pousse que
  > sur changement). Lis *Contracts* (*Mobile FFI*), *Discovery*, *Rule 1 on the LAN* et la section
  > LN13 de `docs/agents/lan-nodes.md`, puis `docs/agents/mobile.md` (*The FFI boundary*, *Threads*,
  > *Rule 1 on the phone*, *MB3b*, *MB15*). Fichiers exclusifs : la ligne LN13. Critères : la liste
  > *Acceptance* de LN13.
  >
  > **Règles (obligatoires, tirées des lots précédents) :**
  > - Lis d'abord `CLAUDE.md`, puis `docs/agents/lan-nodes.md` EN ENTIER, puis les notes que cite ton lot.
  > - Git non interactif : `export GIT_EDITOR=true GIT_SEQUENCE_EDITOR=true EDITOR=true VISUAL=true` ; `git commit -m`, `git merge --no-edit`, jamais `rebase -i`. **Jamais `git stash`** (pile partagée entre worktrees) : une commit WIP à la place.
  > - **Un seul écrivain par fichier** : ne touche QUE les fichiers exclusifs de ton lot (*File ownership*). Besoin d'un autre fichier, ou contrat faux face au code : arrête-toi, `ao report --needs-input`, n'édite pas. Respecte les contrats de la note à la lettre ; si l'un est corrigé après accord, corrige la note dans la même PR.
  > - Type partagé ou manifeste d'une crate partagée (`desktop/crates/*`) modifié : mets à jour **les deux** `Cargo.lock` (`desktop/` et `mobile/rust/`) et attends **les deux** CI vertes (`desktop` et `mobile`).
  > - Mémoire mesurée en **octets comptés** (`Lan::memory()` ou l'équivalent du lot), jamais par la RSS d'un processus partagé.
  > - Un thread qui emprunte un runtime tokio est **joint avant l'arrêt** de ce runtime.
  > - Tests sur des **événements** (canal, `watch`, changement d'état, horloge tokio en pause), jamais sur l'horloge murale ni un `sleep`.
  > - **Jamais l'affichage, le son ni le réseau de l'utilisateur** : Xvfb pour une fenêtre, `mobile/tools/emu.sh` (sans fenêtre ni son) pour l'émulateur, `adb -s emulator-<port>` toujours, tests LAN sur loopback ou dans `unshare -rn`, ports réservés à ton lot (45910 + 10 × ton numéro de lot), tue seulement tes propres PID.
  > - **Rien de gros dans `/tmp`** (tmpfs = RAM) : builds, données de test et journaux sous `target/` du worktree, `~/.cache` ou `~/.local/share`. `CARGO_BUILD_JOBS=4`, un seul cargo à la fois.
  > - Sécurité : un refus ne révèle rien (même code, même chemin), toute taille ou tout compte venu du réseau est plafonné avant allocation, rien du réseau ne devient un chemin, aucune clé, attestation, roster ni cookie dans les journaux. Chaque test de sécurité est lancé une fois contre un mutant (code cassé exprès) et doit alors échouer.
  > - Règle 1 : chaque octet a sa priorité (`Lane` / `Urgency`), le travail de fond cède à la lecture. Copie française, pas d'emoji, jetons du design system.
  > - Mesure chaque critère d'acceptation ; écris les nombres (avec la commande) dans la PR et à côté des assertions.
  > - Avant de fusionner : `git fetch origin && git rebase origin/master`, puis `git diff --stat origin/master...HEAD` ne doit lister QUE les fichiers de ton lot (et sa section de la note) ; sinon base périmée ou édition parasite : corrige avant.
  > - Branche dans l'espace de branches de ta session Orchestrator. PR : résumé, sources, mesures, risques. CI **entièrement** verte (chaque job de chaque workflow, journaux de `security.yaml` lus, chaque nouvelle alerte corrigée). **Une PR verte se fusionne sans attendre ni demander.**
  > - Fin : remplace la section de ton lot dans `docs/agents/lan-nodes.md` par ce que tu as appris (règle, pourquoi, nombre mesuré), fusionne, puis `ao report --done`.

### LN14 — Android: NSD and screens (wave 7, effort high)

- **Objective**: the Android app discovers and announces through `NsdManager`, holds the multicast
  lock only while browsing, follows the app's activity (foreground / playing / downloading) for the
  battery, and has the « Appareils à proximité » screen and the « Sur vos appareils » tab.
- **Files**: LN14 row. **Contracts**: *Discovery* (Android), *Screens and Réglages*, mobile.md's
  standard for every screen (Sillon tokens, `LoadFailed`, French copy, `LazyColumn` over handles).
- **Depends on**: LN13; mobile MB16 merged (the Réglages entry it adds a row to).
- **Acceptance**: on the emulator (`mobile/tools/emu.sh`, headless): the app registers
  `_neezer-lan._udp` with the port `LanApi::advertised_port()` returns and a random name; a node
  pushed into the same emulator is found and resolved (or, if the emulator's loopback carries no
  multicast — measure it — on a real phone over `adb`, said in the PR); desktop ↔ emulator through
  `redir` + last-known address plays a track from the desktop on the phone; the multicast lock is
  held only while browsing (`dumpsys wifi | grep -i multicast` before/after); screen off and not
  playing → NSD stopped and connections closed within 60 s (`dumpsys` + `LanApi.snapshot()`), CPU
  time over 30 min idle with LAN on ≤ +5 % vs off; Compose UI tests by semantics for both screens;
  screenshots (phone, tablet) in `docs/agents/screenshots/lan/LN14/`.
- **Pitfalls**: `NsdManager` callbacks arrive on a binder thread — hop to the core through the FFI,
  never block; `resolveService` allows one resolve at a time before API 34 (queue them); release the
  lock in every path (`try/finally`); never run `adb` without `-s emulator-<port>`.
- **Prompt**:

  > Tu implémentes le lot **LN14 — Android: NSD and screens** du plan « nœuds LAN » (effort high).
  > But : l'app Android découvre et s'annonce par `NsdManager` (`_neezer-lan._udp`, nom aléatoire,
  > port donné par `LanApi`), ne tient le verrou multicast que pendant la recherche, suit l'activité
  > de l'app (premier plan, lecture, téléchargements) pour la batterie, et offre l'écran « Appareils à
  > proximité » et l'onglet « Sur vos appareils ». Lis *Discovery*, *Screens and Réglages*, *Two
  > nodes on one machine* (émulateur) et la section LN14 de `docs/agents/lan-nodes.md`, puis
  > `docs/agents/mobile.md` (*Tests and emulation*, *The standard every screen lot meets*, *MB16*).
  > Fichiers exclusifs : la ligne LN14. Critères : la liste *Acceptance* de LN14, mesurés sur
  > l'émulateur sans fenêtre ni son.
  >
  > **Règles (obligatoires, tirées des lots précédents) :**
  > - Lis d'abord `CLAUDE.md`, puis `docs/agents/lan-nodes.md` EN ENTIER, puis les notes que cite ton lot.
  > - Git non interactif : `export GIT_EDITOR=true GIT_SEQUENCE_EDITOR=true EDITOR=true VISUAL=true` ; `git commit -m`, `git merge --no-edit`, jamais `rebase -i`. **Jamais `git stash`** (pile partagée entre worktrees) : une commit WIP à la place.
  > - **Un seul écrivain par fichier** : ne touche QUE les fichiers exclusifs de ton lot (*File ownership*). Besoin d'un autre fichier, ou contrat faux face au code : arrête-toi, `ao report --needs-input`, n'édite pas. Respecte les contrats de la note à la lettre ; si l'un est corrigé après accord, corrige la note dans la même PR.
  > - Type partagé ou manifeste d'une crate partagée (`desktop/crates/*`) modifié : mets à jour **les deux** `Cargo.lock` (`desktop/` et `mobile/rust/`) et attends **les deux** CI vertes (`desktop` et `mobile`).
  > - Mémoire mesurée en **octets comptés** (`Lan::memory()` ou l'équivalent du lot), jamais par la RSS d'un processus partagé.
  > - Un thread qui emprunte un runtime tokio est **joint avant l'arrêt** de ce runtime.
  > - Tests sur des **événements** (canal, `watch`, changement d'état, horloge tokio en pause), jamais sur l'horloge murale ni un `sleep`.
  > - **Jamais l'affichage, le son ni le réseau de l'utilisateur** : Xvfb pour une fenêtre, `mobile/tools/emu.sh` (sans fenêtre ni son) pour l'émulateur, `adb -s emulator-<port>` toujours, tests LAN sur loopback ou dans `unshare -rn`, ports réservés à ton lot (45910 + 10 × ton numéro de lot), tue seulement tes propres PID.
  > - **Rien de gros dans `/tmp`** (tmpfs = RAM) : builds, données de test et journaux sous `target/` du worktree, `~/.cache` ou `~/.local/share`. `CARGO_BUILD_JOBS=4`, un seul cargo à la fois.
  > - Sécurité : un refus ne révèle rien (même code, même chemin), toute taille ou tout compte venu du réseau est plafonné avant allocation, rien du réseau ne devient un chemin, aucune clé, attestation, roster ni cookie dans les journaux. Chaque test de sécurité est lancé une fois contre un mutant (code cassé exprès) et doit alors échouer.
  > - Règle 1 : chaque octet a sa priorité (`Lane` / `Urgency`), le travail de fond cède à la lecture. Copie française, pas d'emoji, jetons du design system.
  > - Mesure chaque critère d'acceptation ; écris les nombres (avec la commande) dans la PR et à côté des assertions.
  > - Avant de fusionner : `git fetch origin && git rebase origin/master`, puis `git diff --stat origin/master...HEAD` ne doit lister QUE les fichiers de ton lot (et sa section de la note) ; sinon base périmée ou édition parasite : corrige avant.
  > - Branche dans l'espace de branches de ta session Orchestrator. PR : résumé, sources, mesures, risques. CI **entièrement** verte (chaque job de chaque workflow, journaux de `security.yaml` lus, chaque nouvelle alerte corrigée). **Une PR verte se fusionne sans attendre ni demander.**
  > - Fin : remplace la section de ton lot dans `docs/agents/lan-nodes.md` par ce que tu as appris (règle, pourquoi, nombre mesuré), fusionne, puis `ao report --done`.

### LN15 — Security audit, mutants, the two-node journey measured (wave 8, effort max)

- **Objective**: prove the threat model on the real code with two real nodes, measure the whole
  journey against *Measures*, and leave the harness in the repository.
- **Files**: LN15 row.
- **Mutants** (each a test that passes on the real code and FAILS on a deliberately broken copy,
  run once each and listed in the PR): M1 unknown `kid`; M2 expired by 601 s; M3 not yet valid; M4
  one flipped byte in the credential; M5 revoked key; M6 proof over another connection's exporter
  (replay); M7 responder proof replayed as initiator (reflection); M8 another account's device, not a
  friend → refused, **0 bytes** of identity written; M9 a friend's out-of-scope `Fetch`/`Meta`/
  `Cover`/`Sets` → byte-identical to absent; M10 a friend's inventory contains only shared tracks;
  M11 a 1 MiB + 1 frame; M12 `from > len`; M13 resume with another `FileTag`; M14 more bytes than
  `len`; M15 wrong SHA-256 → not published, peer marked bad; M16 forged roster gossiped → ignored;
  M17 lower-serial revocation list → ignored; M18 0-RTT offered → none accepted; M19 a responder that
  never sends `AuthOk` → the initiator gives up at 2 s having sent no request; M20 a stranger
  initiator → the responder writes 0 application bytes; M21 a remote-control session calling every
  `/api/lan/*` route → refused; M22 a friend removed on the server → refused by the owner's devices
  after their next roster sync, and by a device that only met another of the owner's devices
  (roster gossip).
- **Journey** (desktop↔desktop in the netns with `examples/lan_node`; desktop↔emulator): every row of
  *Measures* measured and written into this note and the PR; the travel scenario end to end: PC with
  2 000 downloaded tracks, phone empty, same LAN, server unreachable (the netns has no route to it):
  the phone downloads three playlists from the PC with covers and metadata, plays them with the PC
  gone, and back online with Opus 64-only copies in T it changes nothing (decided per track, as
  specified).
- **Depends on**: every LN lot.
- **Prompt**:

  > Tu implémentes le lot **LN15 — Security audit, mutants, the two-node journey measured** du plan
  > « nœuds LAN » (effort max : sécurité). But : prouver le modèle de menace sur le vrai code avec deux
  > vrais nœuds (les mutants M1 à M22 de la note, chacun lancé une fois contre une copie cassée
  > exprès), mesurer tout le parcours selon *Measures* (bureau↔bureau dans `unshare -rn`, bureau↔
  > émulateur sans fenêtre) dont le scénario du voyage serveur injoignable, laisser le harnais
  > (`examples/lan_node.rs`, `tools/two_nodes.sh`) dans le dépôt, et écrire les nombres mesurés et le
  > tableau final du modèle de menace dans `docs/agents/lan-nodes.md`. Lis toute la note, puis
  > `docs/agents/testing.md` et `docs/agents/commands.md`. Fichiers exclusifs : la ligne LN15.
  >
  > **Règles (obligatoires, tirées des lots précédents) :**
  > - Lis d'abord `CLAUDE.md`, puis `docs/agents/lan-nodes.md` EN ENTIER, puis les notes que cite ton lot.
  > - Git non interactif : `export GIT_EDITOR=true GIT_SEQUENCE_EDITOR=true EDITOR=true VISUAL=true` ; `git commit -m`, `git merge --no-edit`, jamais `rebase -i`. **Jamais `git stash`** (pile partagée entre worktrees) : une commit WIP à la place.
  > - **Un seul écrivain par fichier** : ne touche QUE les fichiers exclusifs de ton lot (*File ownership*). Besoin d'un autre fichier, ou contrat faux face au code : arrête-toi, `ao report --needs-input`, n'édite pas. Respecte les contrats de la note à la lettre ; si l'un est corrigé après accord, corrige la note dans la même PR.
  > - Type partagé ou manifeste d'une crate partagée (`desktop/crates/*`) modifié : mets à jour **les deux** `Cargo.lock` (`desktop/` et `mobile/rust/`) et attends **les deux** CI vertes (`desktop` et `mobile`).
  > - Mémoire mesurée en **octets comptés** (`Lan::memory()` ou l'équivalent du lot), jamais par la RSS d'un processus partagé.
  > - Un thread qui emprunte un runtime tokio est **joint avant l'arrêt** de ce runtime.
  > - Tests sur des **événements** (canal, `watch`, changement d'état, horloge tokio en pause), jamais sur l'horloge murale ni un `sleep`.
  > - **Jamais l'affichage, le son ni le réseau de l'utilisateur** : Xvfb pour une fenêtre, `mobile/tools/emu.sh` (sans fenêtre ni son) pour l'émulateur, `adb -s emulator-<port>` toujours, tests LAN sur loopback ou dans `unshare -rn`, ports réservés à ton lot (45910 + 10 × ton numéro de lot), tue seulement tes propres PID.
  > - **Rien de gros dans `/tmp`** (tmpfs = RAM) : builds, données de test et journaux sous `target/` du worktree, `~/.cache` ou `~/.local/share`. `CARGO_BUILD_JOBS=4`, un seul cargo à la fois.
  > - Sécurité : un refus ne révèle rien (même code, même chemin), toute taille ou tout compte venu du réseau est plafonné avant allocation, rien du réseau ne devient un chemin, aucune clé, attestation, roster ni cookie dans les journaux. Chaque test de sécurité est lancé une fois contre un mutant (code cassé exprès) et doit alors échouer.
  > - Règle 1 : chaque octet a sa priorité (`Lane` / `Urgency`), le travail de fond cède à la lecture. Copie française, pas d'emoji, jetons du design system.
  > - Mesure chaque critère d'acceptation ; écris les nombres (avec la commande) dans la PR et à côté des assertions.
  > - Avant de fusionner : `git fetch origin && git rebase origin/master`, puis `git diff --stat origin/master...HEAD` ne doit lister QUE les fichiers de ton lot (et sa section de la note) ; sinon base périmée ou édition parasite : corrige avant.
  > - Branche dans l'espace de branches de ta session Orchestrator. PR : résumé, sources, mesures, risques. CI **entièrement** verte (chaque job de chaque workflow, journaux de `security.yaml` lus, chaque nouvelle alerte corrigée). **Une PR verte se fusionne sans attendre ni demander.**
  > - Fin : remplace la section de ton lot dans `docs/agents/lan-nodes.md` par ce que tu as appris (règle, pourquoi, nombre mesuré), fusionne, puis `ao report --done`.

## Not in this plan (said, so nobody adds it by accident)

- Exchange between devices of DIFFERENT servers (the trust anchor is one server's LAN key).
- Relays, hole punching, Internet-wide peer-to-peer: a LAN has no NAT between its nodes.
- Upgrading, once the server is back, a download taken from a peer at a quality outside the tolerated
  list (the rule as specified accepts the peer's copy; a later "upgrade" job is a separate decision).
- The web player as a node (no mDNS, no raw QUIC in a browser).
- iOS (the `External` discovery seam and Bonjour notes above are what it will need).
