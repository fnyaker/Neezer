//! Where a track will be taken from: the local disk, a peer, or the server
//! (docs/agents/lan-nodes.md, *Choosing the source*).
//!
//! **LN9 filled it.** It is a pure function over data that is already in memory —
//! the replicated inventories, the local index, the tolerance and the server's
//! reachability — so the answer costs no request and under a millisecond. That is
//! what keeps rule 3 true on a bad link: a tap never waits on the LAN to learn the
//! LAN has nothing.
//!
//! ```text
//! 1. a complete local file (download, then play cache)      -> Local
//! 2. candidates = peers having the track, each with its qualities
//! 3. one has W, or a quality in T                           -> Peer(best)
//!    best = W if present, else the highest quality in T;
//!    ties: own device before friend, then the peer a download is already
//!    committed to (`prefer`), then lowest RTT
//! 4. else the server is reachable                           -> Server(W)
//! 5. else any candidate at all                              -> Peer(highest it has)
//! 6. else                                                   -> Server(W) (fails
//!    fast offline: rule 3)
//! ```
//!
//! Row 5 is the travel case: with no server, a worse copy beats silence, which is
//! why the tolerance only ever applies while the server could answer.
//!
//! # Row 3 is decided on the QUALITY first, then on the peer
//!
//! The note says `best = W if present, else the highest quality in T`, so the
//! quality comes from what the peers offer and only then is a peer chosen among
//! those that can serve it. Asking for Opus 128 while one peer holds FLAC and
//! another Opus 128 therefore serves Opus 128: the ladder asked for that rung, and
//! a peer is not the server that can transcode to any of them.

use std::path::PathBuf;

use crate::peers::Candidate;
use crate::types::{DevicePub, Relation, Source, Tolerance};

/// The decision, from data already in memory.
///
/// `local` is a complete local file if `nz-cache` has one, `prefer` the peer a
/// playlist download is already committed to (so its 50 tracks come from the same
/// device instead of scattering across the LAN).
pub fn decide(
    local: Option<PathBuf>,
    candidates: &[Candidate],
    wanted: nz_api::Quality,
    tolerated: &Tolerance,
    server_reachable: bool,
    prefer: Option<DevicePub>,
) -> Source {
    // Row 1: never send the person to the network for a file already on disk.
    if let Some(path) = local {
        return Source::Local(path);
    }

    // Rows 2 and 3: the quality first (the wanted rung if a peer has it, else the
    // best tolerated one any peer has), then the peer that can serve it.
    let quality = if candidates.iter().any(|candidate| candidate.has(wanted)) {
        Some(wanted)
    } else {
        candidates
            .iter()
            .filter_map(|candidate| tolerated.best_of(candidate.qualities()))
            .max()
    };
    if let Some(quality) = quality
        && let Some(chosen) = candidates
            .iter()
            .filter(|candidate| candidate.has(quality))
            .min_by_key(|candidate| rank(candidate, prefer))
    {
        return Source::Peer {
            peer: chosen.peer,
            quality,
        };
    }

    // Row 4: the server could send the rung that was asked for.
    if server_reachable {
        return Source::Server(wanted);
    }

    // Row 5: no server, so a worse copy beats silence. The best copy wins, and the
    // note's tie-break order decides between two peers that have as good a one.
    let mut best: Option<(&Candidate, nz_api::Quality)> = None;
    for candidate in candidates {
        let Some(quality) = candidate.best_quality() else {
            continue;
        };
        let better = match best {
            None => true,
            Some((chosen, chosen_quality)) => {
                (quality, std::cmp::Reverse(rank(candidate, prefer)))
                    > (chosen_quality, std::cmp::Reverse(rank(chosen, prefer)))
            }
        };
        if better {
            best = Some((candidate, quality));
        }
    }
    if let Some((candidate, quality)) = best {
        return Source::Peer {
            peer: candidate.peer,
            quality,
        };
    }

    // Row 6: nothing anywhere. The server is still the answer, so the caller fails
    // fast with ITS cause (offline, a 404) instead of inventing a LAN verdict.
    Source::Server(wanted)
}

/// The note's tie-break, smallest first: own device before friend, then the peer a
/// download is already committed to, then the lowest round trip, then the device key
/// itself (so two peers that tie on everything still answer in a stable order).
fn rank(candidate: &Candidate, prefer: Option<DevicePub>) -> (u8, u8, u32, DevicePub) {
    let own = u8::from(candidate.relation != Relation::Own);
    let preferred = u8::from(prefer != Some(candidate.peer));
    (own, preferred, candidate.rtt_ms, candidate.peer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nz_api::Quality;

    const PEER: DevicePub = [0x2a; 32];

    fn candidate(peer: DevicePub, relation: Relation, copies: &[Quality]) -> Candidate {
        Candidate {
            peer,
            relation,
            copies: copies.iter().map(|quality| (*quality, 1_000)).collect(),
            rtt_ms: 5,
        }
    }

    fn own(copies: &[Quality]) -> Candidate {
        candidate(PEER, Relation::Own, copies)
    }

    fn friend(copies: &[Quality]) -> Candidate {
        candidate(
            PEER,
            Relation::Friend {
                name: "Alice".to_owned(),
            },
            copies,
        )
    }

    /// Acceptance: "`decide` is exhaustively tested against the six-row table
    /// (every combination of local / W present / in T / out of T / server reachable
    /// or not / no candidate: 48 cases, each asserted)".
    ///
    /// The three wanted rungs × two server states × two local states × four peer
    /// states are exactly 48 cases, and each one is asserted below:
    ///
    /// | local | peers | server | answer |
    /// |---|---|---|---|
    /// | yes | any | any | `Local` |
    /// | no | none | yes / no | `Server(W)` (row 6 is row 4 with nothing to offer) |
    /// | no | has W | any | `Peer(peer, W)` |
    /// | no | has a tolerated rung | any | `Peer(peer, best tolerated)` |
    /// | no | only an untolerated rung | yes | `Server(W)` |
    /// | no | only an untolerated rung | no | `Peer(peer, that rung)` |
    ///
    /// Run: `cargo test -p nz-lan decide`.
    #[test]
    fn every_combination_of_the_source_table_is_asserted() {
        let tolerated = Tolerance::default_set();
        let local = || Some(PathBuf::from("/data/offline/song.flac"));
        let mut cases = 0;

        for wanted in [Quality::Flac, Quality::Opus320, Quality::Opus128] {
            // A tolerated rung that is NOT the wanted one, and a rung the default
            // set refuses (Opus 64, the Auto ladder's bad-link rung).
            let in_t = match wanted {
                Quality::Flac => Quality::Opus320,
                Quality::Opus320 => Quality::Flac,
                _ => Quality::Opus320,
            };
            assert!(tolerated.contains(in_t));
            assert_ne!(in_t, wanted);
            let best_in_t = tolerated.best_of([in_t]).expect("tolerated");

            for server_reachable in [true, false] {
                for has_local in [false, true] {
                    for peers in [
                        None,
                        Some(vec![wanted]),
                        Some(vec![in_t]),
                        Some(vec![Quality::Opus64]),
                    ] {
                        let candidates: Vec<Candidate> = match &peers {
                            None => Vec::new(),
                            Some(copies) => vec![own(copies)],
                        };
                        let answer = decide(
                            has_local.then(local).flatten(),
                            &candidates,
                            wanted,
                            &tolerated,
                            server_reachable,
                            None,
                        );
                        let expected = if has_local {
                            Source::Local(PathBuf::from("/data/offline/song.flac"))
                        } else {
                            match &peers {
                                None => Source::Server(wanted),
                                Some(copies) if copies[0] == wanted => Source::Peer {
                                    peer: PEER,
                                    quality: wanted,
                                },
                                Some(copies) if copies[0] == in_t => Source::Peer {
                                    peer: PEER,
                                    quality: best_in_t,
                                },
                                Some(_) if server_reachable => Source::Server(wanted),
                                Some(copies) => Source::Peer {
                                    peer: PEER,
                                    quality: copies[0],
                                },
                            }
                        };
                        assert_eq!(
                            answer, expected,
                            "local={has_local} peers={peers:?} wanted={wanted:?} \
                             server={server_reachable}"
                        );
                        cases += 1;
                    }
                }
            }
        }
        assert_eq!(cases, 48, "the table has 48 cases");
    }

    /// Acceptance's mutant: "swap rows 4 and 5 → tests fail". Row 4 (server when
    /// reachable) and row 5 (the peer's worse copy when not) are the two rows whose
    /// order the tolerance exists for, so this test states both sides of the hinge
    /// on ONE candidate: reachable answers the server, unreachable answers the peer.
    /// A `decide` that consulted the peer first would answer `Peer` in both.
    #[test]
    fn the_server_wins_while_it_is_reachable_and_the_peer_only_without_it() {
        let tolerated = Tolerance::default_set();
        let candidates = [friend(&[Quality::Opus64])];
        assert_eq!(
            decide(None, &candidates, Quality::Flac, &tolerated, true, None),
            Source::Server(Quality::Flac)
        );
        assert_eq!(
            decide(None, &candidates, Quality::Flac, &tolerated, false, None),
            Source::Peer {
                peer: PEER,
                quality: Quality::Opus64,
            }
        );
    }

    /// Row 3's tie-break, in the note's order: my own device before a friend, then
    /// the peer a download is already committed to, then the lowest round trip.
    #[test]
    fn ties_go_to_my_own_device_then_the_preferred_peer_then_the_lowest_rtt() {
        let tolerated = Tolerance::default_set();
        let mut mine = candidate([0x10; 32], Relation::Own, &[Quality::Flac]);
        mine.rtt_ms = 90;
        let mut hers = candidate(
            [0x20; 32],
            Relation::Friend {
                name: "Alice".to_owned(),
            },
            &[Quality::Flac],
        );
        hers.rtt_ms = 1;
        // Own wins even with the worse link.
        assert_eq!(
            decide(
                None,
                &[hers.clone(), mine.clone()],
                Quality::Flac,
                &tolerated,
                true,
                None
            ),
            Source::Peer {
                peer: [0x10; 32],
                quality: Quality::Flac,
            }
        );

        // Two friends, same copy: the preferred peer (the one a playlist download is
        // committed to) wins, whatever the latency.
        let mut slow = candidate(
            [0x30; 32],
            Relation::Friend {
                name: "Bob".to_owned(),
            },
            &[Quality::Flac],
        );
        slow.rtt_ms = 40;
        assert_eq!(
            decide(
                None,
                &[hers.clone(), slow.clone()],
                Quality::Flac,
                &tolerated,
                true,
                Some([0x30; 32])
            ),
            Source::Peer {
                peer: [0x30; 32],
                quality: Quality::Flac,
            }
        );

        // Neither preferred: the lowest RTT.
        assert_eq!(
            decide(
                None,
                &[hers.clone(), slow],
                Quality::Flac,
                &tolerated,
                true,
                None
            ),
            Source::Peer {
                peer: [0x20; 32],
                quality: Quality::Flac,
            }
        );
    }

    /// Row 5 with two peers of different qualities: the highest copy wins, not the
    /// first candidate, and the tie-break still applies between equal copies.
    #[test]
    fn without_a_server_the_best_copy_available_wins() {
        let tolerated = Tolerance::default_set();
        let low = candidate([0x40; 32], Relation::Own, &[Quality::Opus64]);
        let high = candidate([0x41; 32], Relation::Own, &[Quality::Opus128]);
        assert_eq!(
            decide(
                None,
                &[low.clone(), high.clone()],
                Quality::Flac,
                &tolerated,
                false,
                None
            ),
            Source::Peer {
                peer: [0x41; 32],
                quality: Quality::Opus128,
            }
        );
        assert_eq!(
            decide(None, &[high, low], Quality::Flac, &tolerated, false, None),
            Source::Peer {
                peer: [0x41; 32],
                quality: Quality::Opus128,
            }
        );
    }

    /// Row 3 serves the WANTED rung when a peer has it, even if another peer holds a
    /// higher one: the ladder asked for a rung, and a peer is not a transcoder.
    #[test]
    fn a_peer_that_has_the_wanted_rung_serves_it_rather_than_a_higher_one() {
        let tolerated = Tolerance::default_set();
        let flac = candidate([0x50; 32], Relation::Own, &[Quality::Flac]);
        let asked = candidate([0x51; 32], Relation::Own, &[Quality::Opus128]);
        assert_eq!(
            decide(
                None,
                &[flac, asked],
                Quality::Opus128,
                &tolerated,
                true,
                None
            ),
            Source::Peer {
                peer: [0x51; 32],
                quality: Quality::Opus128,
            }
        );
    }

    /// Acceptance: "the decision itself adds **0 requests** and ≤ 1 ms (in-memory)".
    ///
    /// Sixteen candidates (the connection cap) each holding four copies: the whole
    /// table is walked 10 000 times, so the average is the decision's own cost and
    /// not a scheduler tick. Measured on this machine, debug: ~0.2 µs per call.
    #[test]
    fn the_decision_is_well_under_a_millisecond() {
        let tolerated = Tolerance::default_set();
        let mut candidates = Vec::new();
        for n in 0..16u8 {
            candidates.push(Candidate {
                peer: [n; 32],
                relation: Relation::Own,
                copies: vec![
                    (Quality::Flac, 30_000_000),
                    (Quality::Opus320, 9_000_000),
                    (Quality::Opus128, 4_000_000),
                    (Quality::Opus64, 2_000_000),
                ],
                rtt_ms: u32::from(n),
            });
        }
        let started = std::time::Instant::now();
        const RUNS: u32 = 10_000;
        for _ in 0..RUNS {
            let answer = decide(None, &candidates, Quality::Opus128, &tolerated, true, None);
            assert!(matches!(answer, Source::Peer { .. }));
        }
        let per_call = started.elapsed().as_secs_f64() / f64::from(RUNS);
        println!(
            "decide, 16 candidates, {RUNS} runs, {} build: {:.3} µs per decision",
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            per_call * 1e6
        );
        assert!(
            per_call < 0.001,
            "{:.3} µs per decision is over the note's 1 ms",
            per_call * 1e6
        );
    }
}
