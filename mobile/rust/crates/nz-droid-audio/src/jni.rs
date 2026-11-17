//! The four JNI entry points `AudioTrackPull.kt` declares.
//!
//! Written against the C ABI rather than a binding crate, exactly as
//! `android/nscore/src/jni.rs` is: the two JNIEnv functions the pull needs sit at
//! fixed slots in the interface table — **230 `GetDirectBufferAddress`, 231
//! `GetDirectBufferCapacity`**, counted in the JDK's `jni.h` (the same file
//! `nscore` checked NewStringUTF 167 and NewDirectByteBuffer 229 against) — and
//! every other argument is a primitive.
//!
//! Kotlin declares them `@JvmStatic private external fun`, on
//! `fr.neezer.mobile.playback.AudioTrackPull`, hence the mangled names. A handle
//! is a `Box<PullSession>` turned into a `jlong`; **0 is "none"** and every entry
//! point answers a no-op (`-1` for a render) instead of crashing, so a Kotlin
//! object closed twice, or used after a close, cannot take the process down.

use core::ffi::c_void;

use crate::CHANNELS;
use crate::track::{close_handle, session_mut};

type JEnv = *mut *const *const c_void;
type JClass = *mut c_void;
type JObject = *mut c_void;

const GET_DIRECT_BUFFER_ADDRESS: usize = 230;
const GET_DIRECT_BUFFER_CAPACITY: usize = 231;

unsafe fn env_fn(env: JEnv, slot: usize) -> *const c_void {
    // SAFETY: the JVM passes a valid JNIEnv table for the duration of the call.
    unsafe { *(*env).add(slot) }
}

/// `GetDirectBufferAddress`: the first byte of a direct `ByteBuffer`, or null
/// when the buffer is not direct (which is a bug on the Kotlin side).
unsafe fn direct_address(env: JEnv, buffer: JObject) -> *mut c_void {
    let f: extern "system" fn(JEnv, JObject) -> *mut c_void =
        unsafe { core::mem::transmute(env_fn(env, GET_DIRECT_BUFFER_ADDRESS)) };
    f(env, buffer)
}

/// `GetDirectBufferCapacity`, in bytes.
unsafe fn direct_capacity(env: JEnv, buffer: JObject) -> i64 {
    let f: extern "system" fn(JEnv, JObject) -> i64 =
        unsafe { core::mem::transmute(env_fn(env, GET_DIRECT_BUFFER_CAPACITY)) };
    f(env, buffer)
}

/// `nativeRender(handle: Long, buffer: ByteBuffer, frames: Int): Int` — fill
/// `frames` interleaved stereo frames into the Kotlin thread's direct buffer.
///
/// Returns `frames` on success, `-1` for a handle that is 0/closed, a null or
/// too small buffer, or a non-positive frame count. The Kotlin loop treats `-1`
/// as a fatal output failure.
///
/// # Safety
/// Called by the JVM. `buffer` must be a direct `ByteBuffer`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_fr_neezer_mobile_playback_AudioTrackPull_nativeRender(
    env: JEnv,
    _class: JClass,
    handle: i64,
    buffer: JObject,
    frames: i32,
) -> i32 {
    if frames <= 0 || buffer.is_null() {
        return -1;
    }
    // SAFETY: the handle contract is the Kotlin object's (see the module doc).
    let Some(session) = (unsafe { session_mut(handle) }) else {
        return -1;
    };
    // SAFETY: `env` is the JVM's own table for this call.
    let (address, capacity) =
        unsafe { (direct_address(env, buffer), direct_capacity(env, buffer)) };
    if address.is_null() {
        return -1;
    }
    let samples = frames as i64 * i64::from(CHANNELS);
    if capacity < samples * 4 {
        return -1;
    }
    // SAFETY: the JVM guarantees at least `capacity` bytes at `address`, and the
    // check above proves they hold `samples` f32 values. `Pull::fill` writes
    // exactly `out.len()` samples.
    let out = unsafe { core::slice::from_raw_parts_mut(address.cast::<f32>(), samples as usize) };
    session.fill(out);
    frames
}

/// `nativeSetLatency(handle: Long, frames: Long)` — what the `AudioTrack` holds
/// after a write returns.
///
/// # Safety
/// Called by the JVM with a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_fr_neezer_mobile_playback_AudioTrackPull_nativeSetLatency(
    _env: JEnv,
    _class: JClass,
    handle: i64,
    frames: i64,
) {
    // SAFETY: the handle contract is the Kotlin object's.
    if let Some(session) = unsafe { session_mut(handle) } {
        session.set_latency(frames.max(0) as u64);
    }
}

/// `nativeUnderruns(handle: Long, total: Long)` — `AudioTrack.getUnderrunCount()`
/// as a running total across the tracks this output re-created.
///
/// # Safety
/// Called by the JVM with a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_fr_neezer_mobile_playback_AudioTrackPull_nativeUnderruns(
    _env: JEnv,
    _class: JClass,
    handle: i64,
    total: i64,
) {
    // SAFETY: the handle contract is the Kotlin object's.
    if let Some(session) = unsafe { session_mut(handle) } {
        session.note_underruns(total.max(0) as u64);
    }
}

/// `nativeClose(handle: Long)` — release the session. The Kotlin object calls it
/// once, after its thread has stopped, so no render can be in flight.
///
/// # Safety
/// Called by the JVM; the handle must not be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_fr_neezer_mobile_playback_AudioTrackPull_nativeClose(
    _env: JEnv,
    _class: JClass,
    handle: i64,
) {
    // SAFETY: the handle contract is the Kotlin object's.
    unsafe { close_handle(handle) }
}

/// Keeps the four entry points in the final library.
///
/// `libnz_ffi.so` links this crate as an rlib and resolves these symbols BY NAME
/// at runtime, so a linker that garbage-collects unreferenced sections would drop
/// them and `System.loadLibrary` + `external fun` would fail with
/// `UnsatisfiedLinkError`. These addresses, reachable from a `#[used]` static,
/// are what keeps them — and `examples/jni-export-check.rs` (a cdylib that only
/// depends on this crate) is where that is measured.
#[doc(hidden)]
#[derive(Debug)]
pub struct JniEntryPoints(#[doc(hidden)] pub [*const (); 4]);

// SAFETY: a constant table of code addresses. Nothing dereferences it and
// nothing mutates it; it exists so the linker cannot drop the four symbols.
unsafe impl Sync for JniEntryPoints {}

#[doc(hidden)]
#[used]
pub static JNI_ENTRY_POINTS: JniEntryPoints = JniEntryPoints([
    Java_fr_neezer_mobile_playback_AudioTrackPull_nativeRender as *const (),
    Java_fr_neezer_mobile_playback_AudioTrackPull_nativeSetLatency as *const (),
    Java_fr_neezer_mobile_playback_AudioTrackPull_nativeUnderruns as *const (),
    Java_fr_neezer_mobile_playback_AudioTrackPull_nativeClose as *const (),
]);
