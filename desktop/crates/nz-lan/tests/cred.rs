//! LN2's reference tests: the trust chain's three test vectors, every refusal a
//! credential can earn, and the device key file.
//!
//! Everything here is written from `docs/agents/lan-nodes.md` (*Trust chain*,
//! LN2's *Acceptance*), not from `src/cred.rs`: the layouts, the domain strings,
//! the hex of the three vectors and the caps are the note's numbers, and this file
//! builds its own signed blobs from the reference seeds with `aws-lc-rs`. That is
//! what makes it a test of the server's encoding rather than a test of the
//! decoder's opinion of itself: a change on either side (the Python encoder of
//! `neezer/lan.py`, this decoder) breaks it.
//!
//! The three vectors are the note's own, and an independent implementation
//! (libsodium via PyNaCl) reproduced all three bytes-for-bytes before any Rust was
//! written — see the PR's *Sources and vectors*.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use aws_lc_rs::digest;
use aws_lc_rs::signature::{ED25519, Ed25519KeyPair, UnparsedPublicKey};
use nz_lan::cred::{
    CREDENTIAL_LEN, CredError, Credential, REVOCATIONS_MAX, ROSTER_MAX_BYTES, ROSTER_MAX_FRIENDS,
    ROSTER_MAX_TRACKS, Revocations, Role, Roster, TrustKeys, auth_message,
};
use nz_lan::keys::DeviceKey;
use nz_lan::types::{DevicePub, Kid, Subject};

// --- The note's reference material, verbatim ---------------------------------

/// The server's LAN key seed: bytes `0x00..=0x1f`.
const LAN_SEED: [u8; 32] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
];

/// The device's seed: bytes `0x20..=0x3f`.
const DEVICE_SEED: [u8; 32] = [
    0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f,
    0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f,
];

const LAN_PUB_HEX: &str = "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8";
const DEVICE_PUB_HEX: &str = "29acbae141bccaf0b22e1a94d34d0bc7361e526d0bfe12c89794bc9322966dd7";
const KID_HEX: &str = "56475aa75463474c";
const SUBJECT_HEX: &str = "00112233445566778899aabbccddeeff";
const NOT_BEFORE: i64 = 1_798_761_600;
const NOT_AFTER: i64 = 1_801_353_600;

/// 154 bytes: the credential the server issues for the reference device.
const CREDENTIAL_HEX: &str = concat!(
    "4e5a4c430156475aa75463474c00112233445566778899aabbccddeeff010101010101010101010101",
    "29acbae141bccaf0b22e1a94d34d0bc7361e526d0bfe12c89794bc9322966dd7000000006b36ec80",
    "000000006b5e798003a839e52b155515b9f88b70ab9844585cb0ed07a9a685618cce2243fedfce50",
    "968c222fb9c69068b1ded70f7eff5f1d66acc9419e4f4b524add9fa8bf7efcde09",
);

/// 129 bytes: serial 7, issued at `NOT_BEFORE`, one entry — the reference device.
const REVOCATIONS_HEX: &str = concat!(
    "4e5a4c520156475aa75463474c0000000000000007000000006b36ec800000000129acbae141bccaf0",
    "b22e1a94d34d0bc7361e526d0bfe12c89794bc9322966dd705c36844f72116363d35c16696464d37f6",
    "88ada1a8c22f6a2f40c6a7c718da4def658ef1e6d64f8bbc5308c208fd72f661f7060efdb9dcd17a87",
    "1035e0ebd005",
);

/// 64 bytes: `Ed25519(device, "neezer-lan/1 auth\0" ‖ "initiator" ‖ 0xAA×32)`.
const AUTH_PROOF_HEX: &str = concat!(
    "8cac16e29f13ab9811527b65c630d17c0774e759eff96a937ca365f050f91b6306424a132990da39",
    "4b6082123d0c7825db8adb538ee1b31242a2e036aa78b709",
);

// --- The note's domain strings, written out again on purpose ------------------

const CREDENTIAL_DOMAIN: &[u8] = b"neezer-lan/1 credential\x00";
const REVOCATIONS_DOMAIN: &[u8] = b"neezer-lan/1 revocations\x00";
const ROSTER_DOMAIN: &[u8] = b"neezer-lan/1 roster\x00";
const ENROLL_DOMAIN: &[u8] = b"neezer-lan/1 enroll\x00";

// --- Helpers -----------------------------------------------------------------

fn hex(text: &str) -> Vec<u8> {
    assert_eq!(text.len() % 2, 0, "« {text} » n'est pas hexadécimal");
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    for pair in bytes.as_chunks::<2>().0 {
        let pair = std::str::from_utf8(pair).expect("ascii");
        out.push(u8::from_str_radix(pair, 16).expect("hex"));
    }
    out
}

fn fixed<const N: usize>(text: &str) -> [u8; N] {
    hex(text)
        .try_into()
        .unwrap_or_else(|bytes: Vec<u8>| panic!("{N} octets attendus, {} reçus", bytes.len()))
}

/** The subject whose hex is `value`, zero-padded to 16 bytes. */
fn subject_of(value: u64) -> Subject {
    Subject(fixed::<16>(&format!("{value:032x}")))
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn lan_public() -> [u8; 32] {
    fixed::<32>(LAN_PUB_HEX)
}

fn device_public() -> [u8; 32] {
    fixed::<32>(DEVICE_PUB_HEX)
}

fn reference_kid() -> Kid {
    Kid(fixed::<8>(KID_HEX))
}

/// The keys a device that enrolled against this server holds.
fn lan_keys() -> TrustKeys {
    TrustKeys::from_status(&[(reference_kid(), lan_public())])
}

fn lan_signer() -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&LAN_SEED).expect("the LAN seed")
}

fn device_signer() -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&DEVICE_SEED).expect("the device seed")
}

/// A directory under the crate's own `target/tmp`, never `/tmp` (tmpfs is RAM).
fn temp_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("ln2-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("un répertoire de test");
    dir
}

fn sign(domain: &[u8], message: &[u8]) -> Vec<u8> {
    let mut signed = domain.to_vec();
    signed.extend_from_slice(message);
    lan_signer().sign(&signed).as_ref().to_vec()
}

/// The credential's 90-byte body, laid out by the note's table.
struct Fields {
    kid: [u8; 8],
    subject: [u8; 16],
    device_id: [u8; 12],
    device_pub: [u8; 32],
    not_before: i64,
    not_after: i64,
    flags: u8,
}

fn reference_fields() -> Fields {
    Fields {
        kid: fixed::<8>(KID_HEX),
        subject: fixed::<16>(SUBJECT_HEX),
        device_id: [0x01; 12],
        device_pub: device_public(),
        not_before: NOT_BEFORE,
        not_after: NOT_AFTER,
        flags: 0b11,
    }
}

fn credential_body(fields: &Fields) -> Vec<u8> {
    let mut body = Vec::with_capacity(90);
    body.extend_from_slice(b"NZLC");
    body.push(1);
    body.extend_from_slice(&fields.kid);
    body.extend_from_slice(&fields.subject);
    body.extend_from_slice(&fields.device_id);
    body.extend_from_slice(&fields.device_pub);
    body.extend_from_slice(&fields.not_before.to_be_bytes());
    body.extend_from_slice(&fields.not_after.to_be_bytes());
    body.push(fields.flags);
    assert_eq!(body.len(), 90, "le corps fait 90 octets");
    body
}

fn signed_credential(fields: &Fields) -> Vec<u8> {
    let mut credential = credential_body(fields);
    let signature = sign(CREDENTIAL_DOMAIN, &credential);
    credential.extend_from_slice(&signature);
    assert_eq!(credential.len(), CREDENTIAL_LEN);
    credential
}

fn signed_revocations(serial: u64, issued: i64, keys: &[DevicePub]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"NZLR");
    body.push(1);
    body.extend_from_slice(&fixed::<8>(KID_HEX));
    body.extend_from_slice(&serial.to_be_bytes());
    body.extend_from_slice(&issued.to_be_bytes());
    body.extend_from_slice(&(keys.len() as u32).to_be_bytes());
    for key in keys {
        body.extend_from_slice(key);
    }
    let mut list = body;
    let signature = sign(REVOCATIONS_DOMAIN, &list);
    list.extend_from_slice(&signature);
    list
}

/// The `{"body","sig"}` envelope the server and the gossip carry.
fn roster_envelope(body: &str) -> Vec<u8> {
    let signature = sign(ROSTER_DOMAIN, body.as_bytes());
    format!(
        r#"{{"body":"{}","sig":"{}"}}"#,
        base64(body.as_bytes()),
        base64(&signature)
    )
    .into_bytes()
}

/// Standard base64 with padding — the test's own encoder, so a broken
/// `base64_encode` in the crate cannot agree with it.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let first = u32::from(chunk[0]);
        let second = u32::from(*chunk.get(1).unwrap_or(&0));
        let third = u32::from(*chunk.get(2).unwrap_or(&0));
        let triple = (first << 16) | (second << 8) | third;
        out.push(ALPHABET[(triple >> 18) as usize & 63] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[triple as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// A roster body with `friends` shares and `tracks` ids in the first playlist.
fn roster_body(friends: usize, tracks: usize) -> String {
    let ids = (0..tracks)
        .map(|index| format!("\"t{index:05}\""))
        .collect::<Vec<_>>()
        .join(",");
    let shares = (0..friends)
        .map(|index| {
            format!(
                r#"{{"subject":"{:032x}","all":false,"playlists":[{{"id":"pl{index}","title":"Route","tracks":[{ids}]}}]}}"#,
                index + 1
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"v":1,"kid":"{KID_HEX}","subject":"{SUBJECT_HEX}","serial":42,"issued":{NOT_BEFORE},"expires":{NOT_AFTER},"shares_to":[{shares}],"trusted_by":[{{"subject":"{SUBJECT_HEX}","name":"Alice"}}]}}"#
    )
}

// --- The three test vectors ---------------------------------------------------

/// The note's first vector: the credential decodes to its fields, verifies inside
/// its window, and re-encodes byte for byte (it keeps the exact received bytes).
#[test]
fn the_credential_vector_decodes_verifies_and_re_encodes() {
    let bytes = hex(CREDENTIAL_HEX);
    assert_eq!(bytes.len(), CREDENTIAL_LEN);

    let credential = Credential::decode(&bytes).expect("le vecteur se décode");
    assert_eq!(credential.kid, reference_kid());
    assert_eq!(credential.subject, Subject(fixed::<16>(SUBJECT_HEX)));
    assert_eq!(credential.device_id, [0x01; 12]);
    assert_eq!(credential.device_pub, device_public());
    assert_eq!(credential.not_before, NOT_BEFORE);
    assert_eq!(credential.not_after, NOT_AFTER);
    assert_eq!(credential.flags, 0b11);

    // Re-encode: the 154 bytes received come back unchanged.
    assert_eq!(credential.raw().as_slice(), bytes.as_slice());
    assert_eq!(
        Credential::decode(credential.raw()).expect("re-décodable"),
        credential
    );

    let keys = lan_keys();
    let revoked = Revocations::empty(reference_kid());
    for now in [NOT_BEFORE, NOT_BEFORE + 86_400, NOT_AFTER] {
        credential
            .verify(&keys, &revoked, now)
            .unwrap_or_else(|error| panic!("valide à {now} : {error}"));
    }

    // The note's `kid` definition: SHA-256 of the LAN public key, first 8 bytes.
    let digest = digest::digest(&digest::SHA256, &lan_public());
    assert_eq!(&digest.as_ref()[..8], credential.kid.0.as_slice());
}

/// The note's second vector: the revocation list decodes, verifies, names the
/// reference device, and re-encodes byte for byte.
#[test]
fn the_revocations_vector_decodes_verifies_and_re_encodes() {
    let bytes = hex(REVOCATIONS_HEX);
    // 4 magic + 1 version + 8 kid + 8 serial + 8 issued + 4 count + 32 × 1 + 64 sig.
    assert_eq!(bytes.len(), 33 + 32 + 64);

    let list = Revocations::decode_verify(&bytes, &lan_keys()).expect("le vecteur se vérifie");
    assert_eq!(list.kid, reference_kid());
    assert_eq!(list.serial, 7);
    assert_eq!(list.issued, NOT_BEFORE);
    assert_eq!(list.len(), 1);
    assert!(list.contains(&device_public()));
    assert!(!list.contains(&lan_public()));
    assert_eq!(list.raw(), bytes.as_slice());
}

/// The note's third vector: the auth message is the device key's message, and the
/// proof is exactly the note's 64 bytes.
#[test]
fn the_auth_proof_vector_is_the_device_signature_of_the_exporter() {
    let exporter = [0xAA; 32];
    let message = auth_message(Role::Initiator, &exporter);
    assert_eq!(
        message,
        [b"neezer-lan/1 auth\x00initiator" as &[u8], &exporter].concat()
    );

    let proof = device_signer().sign(&message);
    assert_eq!(to_hex(proof.as_ref()), AUTH_PROOF_HEX);

    // The same 64 bytes through the crate's own key type, loaded from a key file:
    // `keys.rs` and `cred.rs` cannot disagree about the message.
    let dir = temp_dir("auth-proof");
    std::fs::write(dir.join("device.key"), DEVICE_SEED).expect("graine de référence");
    let key = DeviceKey::load_or_create(&dir).expect("la clé se charge");
    assert_eq!(key.public(), device_public());
    assert_eq!(&key.sign(&message)[..], proof.as_ref());

    UnparsedPublicKey::new(&ED25519, device_public())
        .verify(&message, &key.sign(&message))
        .expect("la preuve vérifie contre la clé publique de l'appareil");
}

// --- Every refusal ------------------------------------------------------------

/// One mutant per `CredError` variant: the note's acceptance, variant by variant.
#[test]
fn every_refusal_variant_has_an_input_that_triggers_it() {
    let keys = lan_keys();
    let empty = Revocations::empty(reference_kid());
    let vector = hex(CREDENTIAL_HEX);
    let now = NOT_BEFORE + 86_400;

    // Length: 153, 155 and 0 bytes.
    for bad in [
        vector[..153].to_vec(),
        [vector.clone(), vec![0]].concat(),
        Vec::new(),
    ] {
        assert_eq!(Credential::decode(&bad).err(), Some(CredError::Length));
    }

    // Magic: the first byte of `NZLC` changed.
    let mut bad = vector.clone();
    bad[0] = b'X';
    assert_eq!(Credential::decode(&bad).err(), Some(CredError::Magic));

    // Version: a version this build does not speak.
    let mut bad = vector.clone();
    bad[4] = 2;
    assert_eq!(Credential::decode(&bad).err(), Some(CredError::Version));

    // Flags: a reserved bit, which no version-1 credential may set.
    let mut bad = vector.clone();
    bad[89] = 0b100;
    assert_eq!(Credential::decode(&bad).err(), Some(CredError::Flags));

    // UnknownKid: the vector, verified by a device that knows no key at all.
    let credential = Credential::decode(&vector).expect("le vecteur");
    assert_eq!(
        credential.verify(&TrustKeys::default(), &empty, now).err(),
        Some(CredError::UnknownKid)
    );

    // Signature: one bit of the signature changed.
    let mut bad = vector.clone();
    bad[100] ^= 0x01;
    let mutant = Credential::decode(&bad).expect("la structure est intacte");
    assert_eq!(
        mutant.verify(&keys, &empty, now).err(),
        Some(CredError::Signature)
    );

    // Signature again, the case that matters: signed by ANOTHER key but naming the
    // known kid. A verifier that looked at `kid` alone would accept it.
    let other = Ed25519KeyPair::from_seed_unchecked(&[0x77; 32]).expect("une autre clé");
    let mut body = credential_body(&reference_fields());
    let foreign = other.sign(&[CREDENTIAL_DOMAIN, &body].concat());
    body.extend_from_slice(foreign.as_ref());
    let forged = Credential::decode(&body).expect("la structure est intacte");
    assert_eq!(
        forged.verify(&keys, &empty, now).err(),
        Some(CredError::Signature)
    );

    // NotYetValid / Expired: validly signed, out of the window.
    let mut fields = reference_fields();
    fields.not_before = now + 601;
    let early = Credential::decode(&signed_credential(&fields)).expect("signée");
    assert_eq!(
        early.verify(&keys, &empty, now).err(),
        Some(CredError::NotYetValid)
    );

    let mut fields = reference_fields();
    fields.not_after = now - 601;
    let late = Credential::decode(&signed_credential(&fields)).expect("signée");
    assert_eq!(
        late.verify(&keys, &empty, now).err(),
        Some(CredError::Expired)
    );

    // Revoked: the vector's own revocation list names its device key.
    let list = Revocations::decode_verify(&hex(REVOCATIONS_HEX), &keys).expect("la liste");
    assert_eq!(
        credential.verify(&keys, &list, now).err(),
        Some(CredError::Revoked)
    );

    // Json: an envelope that is not JSON, a body that is not JSON, a bad base64 and
    // a `kid` that is not 16 hex characters.
    assert!(matches!(
        Roster::decode_verify(b"pas du json", &keys).err(),
        Some(CredError::Json(_))
    ));
    assert!(matches!(
        Roster::decode_verify(br#"{"body":"@@@","sig":"@@@"}"#, &keys).err(),
        Some(CredError::Json(_))
    ));
    let signature = sign(ROSTER_DOMAIN, b"pas du json");
    let garbage_body = format!(
        r#"{{"body":"{}","sig":"{}"}}"#,
        base64(b"pas du json"),
        base64(&signature)
    );
    assert!(matches!(
        Roster::decode_verify(garbage_body.as_bytes(), &keys).err(),
        Some(CredError::Json(_))
    ));
    // A signature that is not 64 bytes is a length problem, checked before the body
    // is even looked at.
    assert_eq!(
        Roster::decode_verify(br#"{"body":"cGFzIGR1IGpzb24=","sig":"AAAA"}"#, &keys).err(),
        Some(CredError::Length)
    );
    assert!(matches!(
        Roster::decode_verify(&roster_envelope(r#"{"v":1,"kid":"zz"}"#), &keys).err(),
        Some(CredError::Json(_))
    ));

    // Caps: a revocation list that claims more keys than the cap, and a roster over
    // its two caps.
    let mut oversized = signed_revocations(8, NOT_BEFORE, &[device_public()]);
    oversized[29..33].copy_from_slice(&(REVOCATIONS_MAX as u32 + 1).to_be_bytes());
    assert_eq!(
        Revocations::decode_verify(&oversized, &keys).err(),
        Some(CredError::Caps)
    );
    assert_eq!(
        Roster::decode_verify(
            &roster_envelope(&roster_body(ROSTER_MAX_FRIENDS + 1, 1)),
            &keys
        )
        .err(),
        Some(CredError::Caps)
    );
    assert_eq!(
        Roster::decode_verify(
            &roster_envelope(&roster_body(1, ROSTER_MAX_TRACKS + 1)),
            &keys
        )
        .err(),
        Some(CredError::Caps)
    );
    assert_eq!(
        Roster::decode_verify(&vec![b' '; ROSTER_MAX_BYTES + 1], &keys).err(),
        Some(CredError::Caps)
    );
}

/// The note's headline number: every one of the 154 bytes, flipped in turn, is
/// refused — 154 mutants, 154 refusals, 0 acceptances.
#[test]
fn every_single_byte_flip_of_the_vector_is_refused() {
    let keys = lan_keys();
    let empty = Revocations::empty(reference_kid());
    let now = NOT_BEFORE + 86_400;
    let vector = hex(CREDENTIAL_HEX);

    // The control first: the unmutated vector is accepted, so the count below is a
    // count of refusals and not the count of a broken fixture.
    Credential::decode(&vector)
        .expect("le vecteur")
        .verify(&keys, &empty, now)
        .expect("le vecteur est accepté");

    let mut refused = 0usize;
    for index in 0..CREDENTIAL_LEN {
        let mut mutant = vector.clone();
        mutant[index] ^= 0x01;
        match Credential::decode(&mutant)
            .and_then(|credential| credential.verify(&keys, &empty, now))
        {
            Err(_) => refused += 1,
            Ok(()) => panic!("l'octet {index} modifié a été ACCEPTÉ"),
        }
    }
    assert_eq!(refused, 154, "154 mutants, 154 refusés");
    assert_eq!(refused, CREDENTIAL_LEN);
}

/// The ±10 min window, on both ends, at the exact second.
#[test]
fn the_skew_window_is_six_hundred_seconds_on_both_ends() {
    let keys = lan_keys();
    let empty = Revocations::empty(reference_kid());
    let credential = Credential::decode(&hex(CREDENTIAL_HEX)).expect("le vecteur");

    // `not_before − 600` is accepted, `− 601` is not.
    credential
        .verify(&keys, &empty, NOT_BEFORE - 600)
        .expect("600 s d'avance sont tolérées");
    assert_eq!(
        credential.verify(&keys, &empty, NOT_BEFORE - 601).err(),
        Some(CredError::NotYetValid)
    );
    // `not_after + 600` is accepted, `+ 601` is not.
    credential
        .verify(&keys, &empty, NOT_AFTER + 600)
        .expect("600 s de retard sont tolérées");
    assert_eq!(
        credential.verify(&keys, &empty, NOT_AFTER + 601).err(),
        Some(CredError::Expired)
    );
}

/// A revocation list must be canonical: strictly ascending, so neither a shuffle
/// nor a duplicate can name the same keys twice.
#[test]
fn an_unsorted_or_duplicated_revocation_list_is_refused() {
    let keys = lan_keys();
    let low = fixed::<32>(LAN_PUB_HEX);
    let high = fixed::<32>(DEVICE_PUB_HEX);
    assert!(low < high, "the two keys have a known order");

    // Control: ascending and unique, accepted.
    let good = Revocations::decode_verify(&signed_revocations(9, NOT_BEFORE, &[low, high]), &keys)
        .expect("une liste triée est acceptée");
    assert_eq!(good.len(), 2);
    assert!(good.contains(&low) && good.contains(&high));

    assert_eq!(
        Revocations::decode_verify(&signed_revocations(9, NOT_BEFORE, &[high, low]), &keys).err(),
        Some(CredError::Signature)
    );
    assert_eq!(
        Revocations::decode_verify(&signed_revocations(9, NOT_BEFORE, &[low, low]), &keys).err(),
        Some(CredError::Signature)
    );

    // A maximal list: 4 096 keys, sorted — counted bytes are 33 + 32 × 4 096 + 64
    // = 131 169 for the signed blob, and the decoded list holds one copy of it plus
    // its key array (131 072), so 262 241 counted bytes at the very worst.
    let maximal: Vec<DevicePub> = (0..REVOCATIONS_MAX)
        .map(|index| {
            let mut key = [0u8; 32];
            key[30..].copy_from_slice(&(index as u16).to_be_bytes());
            key
        })
        .collect();
    let list = Revocations::decode_verify(&signed_revocations(10, NOT_BEFORE, &maximal), &keys)
        .expect("une liste au plafond est acceptée");
    assert_eq!(list.len(), REVOCATIONS_MAX);
    assert_eq!(list.raw().len(), 33 + 32 * REVOCATIONS_MAX + 64);
    assert!(list.contains(&maximal[REVOCATIONS_MAX - 1]));
}

/// The roster's caps are the note's, and the boundary itself is accepted.
#[test]
fn a_roster_at_its_caps_is_accepted_and_one_over_is_refused() {
    let keys = lan_keys();

    // 64 friends, one track each: at the friends' cap, accepted.
    let friends = roster_body(ROSTER_MAX_FRIENDS, 1);
    let roster = Roster::decode_verify(&roster_envelope(&friends), &keys)
        .expect("64 amis sont dans le plafond");
    assert_eq!(roster.shares_to.len(), ROSTER_MAX_FRIENDS);
    assert_eq!(roster.raw(), friends.as_bytes());

    // 20 000 track ids in one playlist: at the ids' cap, accepted (a ~200 KB body
    // for 20 000 ids, the note's own scale).
    let tracks = roster_body(1, ROSTER_MAX_TRACKS);
    assert!(tracks.len() < ROSTER_MAX_BYTES);
    let roster = Roster::decode_verify(&roster_envelope(&tracks), &keys)
        .expect("20 000 titres sont dans le plafond");
    assert_eq!(
        roster.shares_to[0].playlists[0].tracks.len(),
        ROSTER_MAX_TRACKS
    );
    assert_eq!(roster.raw().len(), tracks.len());
}

/// A roster is signed over the bytes received: another JSON writer's spacing and
/// key order still verify, and re-serialising it breaks the signature — which is
/// why no verifier may ever re-encode what it verifies.
#[test]
fn the_exact_received_bytes_are_signed_not_a_re_serialisation() {
    let keys = lan_keys();
    // A different writer: keys in another order, extra spaces, an unknown field.
    let body = format!(
        "{{ \"subject\" : \"{SUBJECT_HEX}\",\n  \"issued\":{NOT_BEFORE}, \"serial\":42, \
         \"expires\":{NOT_AFTER}, \"v\":1, \"kid\":\"{KID_HEX}\", \"future\":[1,2],\n  \
         \"trusted_by\":[{{\"name\":\"Alice\",\"subject\":\"{SUBJECT_HEX}\"}}] }}"
    );
    let roster = Roster::decode_verify(&roster_envelope(&body), &keys)
        .expect("les octets reçus sont ce qui est signé");
    assert_eq!(roster.subject, Subject(fixed::<16>(SUBJECT_HEX)));
    assert_eq!(roster.serial, 42);
    assert_eq!(
        roster
            .truster(&Subject(fixed::<16>(SUBJECT_HEX)))
            .map(|t| t.name.as_str()),
        Some("Alice")
    );
    assert_eq!(roster.raw(), body.as_bytes());

    // The same fields, re-serialised compactly, with the original signature: the
    // exact failure the note warns about (a re-encoding of a valid roster).
    let reserialised = format!(
        r#"{{"v":1,"kid":"{KID_HEX}","subject":"{SUBJECT_HEX}","serial":42,"issued":{NOT_BEFORE},"expires":{NOT_AFTER},"shares_to":[],"trusted_by":[{{"subject":"{SUBJECT_HEX}","name":"Alice"}}]}}"#
    );
    let signature = sign(ROSTER_DOMAIN, body.as_bytes());
    let broken = format!(
        r#"{{"body":"{}","sig":"{}"}}"#,
        base64(reserialised.as_bytes()),
        base64(&signature)
    );
    assert_eq!(
        Roster::decode_verify(broken.as_bytes(), &keys).err(),
        Some(CredError::Signature)
    );

    // And one character changed in the signed body, with a rebuilt envelope: still
    // the layout, still the `kid`, no longer the server's bytes.
    let mutated = body.replace("Alice", "Alicf");
    assert_ne!(mutated, body);
    let broken = format!(
        r#"{{"body":"{}","sig":"{}"}}"#,
        base64(mutated.as_bytes()),
        base64(&signature)
    );
    assert_eq!(
        Roster::decode_verify(broken.as_bytes(), &keys).err(),
        Some(CredError::Signature)
    );
}

/// The roster's shares and trusters, and the gossip blob: `raw_signed` rebuilds an
/// envelope a peer verifies to the same body and the same signature.
#[test]
fn a_roster_carries_its_shares_and_gossips_back_as_the_same_body() {
    let keys = lan_keys();
    let body = format!(
        r#"{{"v":1,"kid":"{KID_HEX}","subject":"{SUBJECT_HEX}","serial":42,"issued":{NOT_BEFORE},"expires":{NOT_AFTER},"shares_to":[{{"subject":"{:032x}","all":false,"playlists":[{{"id":"pl-1","title":"Route","tracks":["t1","t2"]}}]}},{{"subject":"{:032x}","all":true,"playlists":[]}}],"trusted_by":[{{"subject":"{:032x}","name":"Alice"}}]}}"#,
        0x11, 0x22, 0x33
    );
    let roster = Roster::decode_verify(&roster_envelope(&body), &keys).expect("le roster");
    assert_eq!(roster.shares_to.len(), 2);
    assert!(!roster.shares_to[0].all);
    assert_eq!(roster.shares_to[0].playlists[0].title, "Route");
    assert_eq!(roster.shares_to[0].playlists[0].tracks, ["t1", "t2"]);
    assert!(roster.shares_to[1].all);
    assert_eq!(roster.shares_to[1].playlists.len(), 0);
    assert_eq!(
        roster.share_to(&subject_of(0x11)).map(|share| share.all),
        Some(false)
    );
    assert_eq!(
        roster.share_to(&subject_of(0x22)).map(|share| share.all),
        Some(true)
    );
    assert_eq!(roster.share_to(&subject_of(0x44)), None);

    let gossiped = roster.raw_signed();
    let verified = Roster::decode_verify(&gossiped, &keys).expect("le blob gossipé");
    assert_eq!(verified.raw(), roster.raw());
    assert_eq!(verified, roster);
}

// --- Counted allocations ------------------------------------------------------

thread_local! {
    /// Whether the current thread is counting its allocations.
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    /// The bytes the current thread allocated while counting.
    static COUNTED: Cell<usize> = const { Cell::new(0) };
}

/// Counts the bytes a thread allocates, so "allocates nothing proportional to the
/// input" is a number and not an opinion.
///
/// The only `unsafe` of this lot lives here, in the test harness: Rust has no safe
/// way to observe the allocator, and `src/cred.rs`/`src/keys.rs` are held to zero
/// `unsafe` by `the_two_source_files_contain_no_unsafe_block`. The counter is
/// thread-local with `const` initialisers, so the allocator never re-enters it and
/// no other test's allocations are counted.
struct CountingAllocator;

fn note(size: usize) {
    let _ = COUNTING.try_with(|counting| {
        if counting.get() {
            let _ = COUNTED.try_with(|counted| counted.set(counted.get() + size));
        }
    });
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        note(size);
        unsafe { System.realloc(pointer, layout, size) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Runs `body` with the allocator counter armed on this thread, and answers what it
/// allocated.
fn counted<R>(body: impl FnOnce() -> R) -> (R, usize) {
    COUNTED.with(|counted| counted.set(0));
    COUNTING.with(|counting| counting.set(true));
    let out = body();
    COUNTING.with(|counting| counting.set(false));
    (out, COUNTED.with(Cell::get))
}

/// One mebibyte of deterministic pseudo-random bytes, allocated before counting.
fn noise(len: usize) -> Vec<u8> {
    let mut state = 0x9e37_79b9_u32;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect()
}

/// The note's allocation criterion, with a control: a 1 MiB blob costs `decode`
/// nothing, while copying it costs exactly its length.
#[test]
fn decode_allocates_nothing_for_a_bad_length() {
    let bytes = noise(1 << 20);
    assert_eq!(bytes.len(), 1_048_576);

    // Control: the counter counts, and a copy of the blob is 1 MiB of it.
    let (copy, copied) = counted(|| bytes.clone());
    assert!(
        copied >= bytes.len(),
        "{copied} octets comptés pour une copie"
    );
    drop(copy);

    // The credential: length first, so nothing proportional to the input.
    let (verdict, allocated) = counted(|| Credential::decode(&bytes));
    assert_eq!(verdict.err(), Some(CredError::Length));
    assert_eq!(allocated, 0, "{allocated} octets alloués pour 1 Mio refusé");

    // The revocation list: its header check comes first too.
    let keys = lan_keys();
    let (verdict, allocated) = counted(|| Revocations::decode_verify(&bytes, &keys));
    assert!(
        matches!(
            verdict.err(),
            Some(CredError::Magic) | Some(CredError::Length)
        ),
        "un blob aléatoire est refusé par sa magie ou sa longueur"
    );
    assert_eq!(allocated, 0, "{allocated} octets alloués pour 1 Mio refusé");
}

// --- The device key file -------------------------------------------------------

#[cfg(unix)]
fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .expect("les métadonnées")
        .permissions()
        .mode()
        & 0o777
}

/// `device.key` is a 32-byte seed at `0600`, created atomically, rotated, and
/// wiped — the whole of the note's key-file acceptance.
#[test]
fn the_key_file_is_a_0600_seed_created_atomically_rotated_and_wiped() {
    let root = temp_dir("key-file");
    let dir = root.join("lan");

    // Creation: the directory is ours only, the file holds the seed and nothing
    // else, and the permissions are the contract's.
    let key = DeviceKey::load_or_create(&dir).expect("la clé se crée");
    let path = dir.join("device.key");
    assert_eq!(std::fs::read(&path).expect("le fichier existe").len(), 32);
    #[cfg(unix)]
    {
        assert_eq!(mode_of(&path), 0o600, "device.key est en 0600");
        assert_eq!(mode_of(&dir), 0o700, "le répertoire lan est en 0700");
    }

    // A second load is the same key: a key that exists is never replaced, and its
    // file is not rewritten.
    let before = std::fs::read(&path).expect("le fichier");
    let again = DeviceKey::load_or_create(&dir).expect("la clé se recharge");
    assert_eq!(again.public(), key.public());
    assert_eq!(std::fs::read(&path).expect("le fichier"), before);

    // Rotation: a new key, same file name, same permissions, one file only.
    let rotated = DeviceKey::rotate(&dir).expect("la clé tourne");
    assert_ne!(rotated.public(), key.public());
    #[cfg(unix)]
    assert_eq!(mode_of(&path), 0o600);
    let names: Vec<String> = std::fs::read_dir(&dir)
        .expect("lisible")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        vec!["device.key".to_owned()],
        "un seul fichier, pas de reste"
    );

    #[cfg(unix)]
    {
        // A key restored from a backup with wider permissions is tightened when it
        // is read: the post-condition is "a 0600 key file".
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        DeviceKey::load_or_create(&dir).expect("la clé se recharge");
        assert_eq!(mode_of(&path), 0o600, "les permissions sont resserrées");
    }

    // A truncated temp file (a write that crashed before its rename) is IGNORED:
    // the key that loads is the one `device.key` holds.
    let temp = dir.join(".device.key.tmp-9999-abcdef");
    std::fs::write(&temp, &DEVICE_SEED[..17]).expect("un temporaire tronqué");
    let loaded = DeviceKey::load_or_create(&dir).expect("la clé se charge malgré le temporaire");
    assert_eq!(loaded.public(), rotated.public());
    #[cfg(unix)]
    assert_eq!(
        mode_of(&temp) & 0o600,
        0o600,
        "un load n'efface pas le temporaire"
    );

    // Wipe removes both the key and the seed a crashed write left behind.
    DeviceKey::wipe(&dir).expect("l'effacement");
    assert!(!path.exists(), "device.key a disparu");
    assert!(!temp.exists(), "le temporaire a disparu aussi");
    DeviceKey::wipe(&dir).expect("effacer une clé absente réussit");
}

/// A key file of the wrong length is an error, never a silent new identity: a
/// device that loses its key becomes a different device for every peer.
#[test]
fn a_damaged_key_file_is_refused_and_never_replaced() {
    let root = temp_dir("damaged-key");
    let path = root.join("device.key");

    for damaged in [vec![0u8; 31], vec![0u8; 33], Vec::new()] {
        std::fs::write(&path, &damaged).expect("un fichier abîmé");
        let error = DeviceKey::load_or_create(&root).expect_err("refusé");
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert_eq!(
            std::fs::read(&path).expect("le fichier est resté"),
            damaged,
            "rien n'a été réécrit"
        );
    }
}

/// A key that reaches a log is a leaked key: `Debug` prints the public half only.
#[test]
fn the_debug_of_a_key_shows_its_public_half_and_never_the_seed() {
    let dir = temp_dir("debug-key");
    std::fs::write(dir.join("device.key"), DEVICE_SEED).expect("graine de référence");
    let key = DeviceKey::load_or_create(&dir).expect("la clé");
    let printed = format!("{key:?}");
    assert!(
        printed.contains(DEVICE_PUB_HEX),
        "la clé publique est lisible : {printed}"
    );
    assert!(
        !printed.contains(&to_hex(&DEVICE_SEED)),
        "la graine est absente : {printed}"
    );
    assert!(!printed.contains("seed"));
}

/// Rotating while a reader reads: every read is a whole 32-byte key, never a
/// truncated one. This is the observable form of "a crash mid-write leaves the old
/// key": with a direct write the reader sees a 0-byte file in the truncation
/// window, and this test fails.
#[test]
fn a_reader_never_sees_half_a_key_while_it_rotates() {
    let dir = temp_dir("rotate-reader").join("lan");
    DeviceKey::load_or_create(&dir).expect("la clé initiale");
    let path = dir.join("device.key");

    let stop = Arc::new(AtomicBool::new(false));
    let reader = {
        let stop = Arc::clone(&stop);
        let path = path.clone();
        std::thread::spawn(move || {
            let mut reads = 0u64;
            while !stop.load(Ordering::Acquire) {
                let bytes = std::fs::read(&path).expect("device.key existe toujours");
                assert_eq!(bytes.len(), 32, "un lecteur a vu {} octets", bytes.len());
                reads += 1;
            }
            reads
        })
    };

    for _ in 0..100 {
        DeviceKey::rotate(&dir).expect("la clé tourne");
    }
    stop.store(true, Ordering::Release);
    let reads = reader.join().expect("le lecteur");
    assert!(
        reads > 0,
        "{reads} lectures pendant 100 rotations, toutes entières"
    );
}

/// `DeviceKey` is what every later lot holds in a shared state: it must cross
/// threads, and `Send + Sync` is the whole reason the pair is kept behind `&self`.
#[test]
fn the_device_key_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<DeviceKey>();
}

/// The enrolment proof's exact message, built here from the note's formula: the
/// domain, the subject hex the server sent, this key's public hex, and `at` in
/// decimal. The same proof over the 8-byte big-endian `at` must NOT verify, which
/// is how the two candidate encodings are told apart (LN1 and LN7 follow this one).
#[test]
fn the_enroll_proof_signs_the_note_message() {
    let dir = temp_dir("enroll-proof");
    std::fs::write(dir.join("device.key"), DEVICE_SEED).expect("graine de référence");
    let key = DeviceKey::load_or_create(&dir).expect("la clé");
    let at = NOT_BEFORE;

    let proof = key.enroll_proof(SUBJECT_HEX, at);
    let mut message = ENROLL_DOMAIN.to_vec();
    message.extend_from_slice(SUBJECT_HEX.as_bytes());
    message.extend_from_slice(DEVICE_PUB_HEX.as_bytes());
    message.extend_from_slice(at.to_string().as_bytes());
    UnparsedPublicKey::new(&ED25519, device_public())
        .verify(&message, &proof)
        .expect("la preuve de possession signe le message de la note");

    let mut binary_at = ENROLL_DOMAIN.to_vec();
    binary_at.extend_from_slice(SUBJECT_HEX.as_bytes());
    binary_at.extend_from_slice(DEVICE_PUB_HEX.as_bytes());
    binary_at.extend_from_slice(&at.to_be_bytes());
    assert!(
        UnparsedPublicKey::new(&ED25519, device_public())
            .verify(&binary_at, &proof)
            .is_err(),
        "`at` est décimal, pas i64 big-endian : les deux messages diffèrent"
    );

    // A different subject, a different instant, a different key: three different
    // proofs. A proof is not a bearer token.
    assert_ne!(proof, key.enroll_proof(&format!("{SUBJECT_HEX}00"), at));
    assert_ne!(proof, key.enroll_proof(SUBJECT_HEX, at + 1));
    let other_dir = temp_dir("enroll-proof-other");
    std::fs::write(other_dir.join("device.key"), [0x5a; 32]).expect("une autre graine");
    let other = DeviceKey::load_or_create(&other_dir).expect("l'autre clé");
    assert_ne!(proof, other.enroll_proof(SUBJECT_HEX, at));
}

/// Zero `unsafe` in the two files this lot owns. The check is on the KEYWORD (a
/// doc comment that says "no `unsafe`" is not a block), because a crate-level
/// `#![forbid(unsafe_code)]` would have to be added to `lib.rs`, which is not this
/// lot's file.
#[test]
fn the_two_source_files_contain_no_unsafe_block() {
    for (name, source) in [
        ("cred.rs", include_str!("../src/cred.rs")),
        ("keys.rs", include_str!("../src/keys.rs")),
    ] {
        for (number, line) in source.lines().enumerate() {
            let line = line.trim_start();
            let unsafe_keyword = ["unsafe {", "unsafe fn", "unsafe impl", "unsafe extern"]
                .iter()
                .any(|pattern| line.contains(pattern));
            assert!(
                !unsafe_keyword,
                "{name}:{} contient un bloc unsafe : {line}",
                number + 1
            );
        }
    }
}

/// The counted size of a decoded credential: it is a fixed 240 bytes on the stack
/// and allocates nothing, so a device's trust state does not grow with its
/// library.
#[test]
fn a_decoded_credential_is_a_fixed_size_and_allocates_nothing() {
    // 8 + 16 + 12 + 32 + 8 + 8 + 1 + 154 = 239, rounded up to the 8-byte alignment
    // of `i64`: 240 bytes, no heap.
    assert_eq!(std::mem::size_of::<Credential>(), 240);

    let bytes = hex(CREDENTIAL_HEX);
    let (credential, allocated) = counted(|| Credential::decode(&bytes));
    let credential = credential.expect("le vecteur");
    assert_eq!(
        allocated, 0,
        "{allocated} octets de tas pour une attestation"
    );
    assert_eq!(credential.raw().len(), 154);
    // kid 8 + serial 8 + issued 8 + two `Vec`s of 24: 72 counted bytes, the list's
    // own bytes excluded (they are `raw`, and they are what limits `len`).
    assert_eq!(std::mem::size_of::<Revocations>(), 72);
}
