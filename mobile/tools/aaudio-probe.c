// mobile/tools/aaudio-probe.c — does this device give AAudio a clock?
//
// The underrun and battery measures (docs/agents/mobile.md, "Measures") are only
// meaningful on a device whose audio output really consumes frames, and the
// emulator is the awkward case: `emu.sh` starts it with `-no-audio` (nothing on
// the person's speakers), and the note asks MQ1 to say whether AAudio still
// opens and CLOCKS with that flag, or whether `-audio none` is needed for the
// measures. "It booted" answers nothing: a stream that opens and then advances
// 0 frames per second reports 0 underruns for the wrong reason.
//
// So this probe is the measure itself: it opens one output stream exactly as the
// app's engine will (PowerSaving performance mode, music usage, 48 kHz stereo
// float, a 1 s buffer — the plan's "big buffers, few wake-ups", never low
// latency), writes silence for N seconds from a real-time callback, and prints
// what only the audio HAL can answer:
//
//   opened        AAudioStreamBuilder_openStream's result (AAUDIO_OK = 0)
//   rate          the sample rate the device actually chose
//   burst_frames  frames per callback (what the engine's render size becomes)
//   buffer_frames the buffer AAudio gave us (the 500 ms–1 s the plan wants)
//   callbacks     how many times our callback ran
//   frames        frames written in total
//   ticks         frames / (rate × elapsed). The FIRST callback pre-fills the
//                 whole buffer at once, so a short run reads ~1 + buffer/elapsed
//                 (1.20 for a 5 s run) without the clock being wrong at all.
//   ticks_steady  (frames − buffer) / (rate × elapsed): the same ratio with the
//                 prefill removed. 1.000 = the HAL consumes at real time.
//   xruns         AAudioStream_getXRunCount (a real measure, not an estimate)
//
// It writes JSON on stdout so `measure.sh underruns` can store it beside the
// other measures without parsing prose. Compiled and pushed by measure.sh with
// the NDK's clang for API 28 (the two setters below — usage and content type —
// were added in 28; the emulator is 35, and the app itself opens AAudio from 27
// with the defaults the platform picks, which is what this probe measures when
// they are absent). Nothing of it is installed on the device permanently.
//
// Silence, not a tone: the person's speakers must stay silent (worker rule 7),
// and a null/blackholed sink tells us nothing about amplitude anyway.

#include <aaudio/AAudio.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static atomic_ullong g_callbacks = 0;
static atomic_ullong g_frames = 0;

static double now_s(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1e9;
}

// Real-time callback: no allocation, no lock, no syscall — the rule the app's own
// sink must follow, and the probe proves the shape is possible on this HAL.
static aaudio_data_callback_result_t on_audio(
    AAudioStream *stream, void *userdata, void *audio_data, int32_t num_frames) {
    (void)stream;
    (void)userdata;
    const int32_t channels = 2;
    memset(audio_data, 0, (size_t)num_frames * channels * sizeof(float));
    atomic_fetch_add(&g_callbacks, 1);
    atomic_fetch_add(&g_frames, (unsigned long long)num_frames);
    return AAUDIO_CALLBACK_RESULT_CONTINUE;
}

int main(int argc, char **argv) {
    const int seconds = (argc > 1) ? atoi(argv[1]) : 5;
    const char *mode = (argc > 2) ? argv[2] : "unspecified";

    AAudioStreamBuilder *builder = NULL;
    aaudio_result_t result = AAudio_createStreamBuilder(&builder);
    if (result != AAUDIO_OK || builder == NULL) {
        printf("{\"opened\":false,\"error\":\"createStreamBuilder\",\"code\":%d,\"mode\":\"%s\"}\n", result, mode);
        return 1;
    }

    const int32_t rate = 48000;
    const int32_t channels = 2;
    AAudioStreamBuilder_setDirection(builder, AAUDIO_DIRECTION_OUTPUT);
    AAudioStreamBuilder_setPerformanceMode(builder, AAUDIO_PERFORMANCE_MODE_POWER_SAVING);
    AAudioStreamBuilder_setUsage(builder, AAUDIO_USAGE_MEDIA);
    AAudioStreamBuilder_setContentType(builder, AAUDIO_CONTENT_TYPE_MUSIC);
    AAudioStreamBuilder_setFormat(builder, AAUDIO_FORMAT_PCM_FLOAT);
    AAudioStreamBuilder_setChannelCount(builder, channels);
    AAudioStreamBuilder_setSampleRate(builder, rate);
    AAudioStreamBuilder_setDataCallback(builder, on_audio, NULL);
    // One second of buffer: what the engine asks for (the old app's measured
    // NativeSink lesson — a music player wants big buffers, not low latency).
    AAudioStreamBuilder_setBufferCapacityInFrames(builder, rate);

    AAudioStream *stream = NULL;
    result = AAudioStreamBuilder_openStream(builder, &stream);
    if (result != AAUDIO_OK || stream == NULL) {
        printf("{\"opened\":false,\"error\":\"openStream\",\"code\":%d,\"mode\":\"%s\"}\n", result, mode);
        AAudioStreamBuilder_delete(builder);
        // AAUDIO_ERROR_UNAVAILABLE (-899) is what a HAL with no output device
        // answers; that number is the answer MQ1 needs, so the exit code is
        // non-zero but the JSON is still printed.
        return 2;
    }

    const int32_t got_rate = AAudioStream_getSampleRate(stream);
    const int32_t burst = AAudioStream_getFramesPerBurst(stream);
    const int32_t buffer_frames = AAudioStream_getBufferCapacityInFrames(stream);

    const double t0 = now_s();
    result = AAudioStream_requestStart(stream);
    if (result != AAUDIO_OK) {
        printf("{\"opened\":true,\"error\":\"requestStart\",\"code\":%d,\"mode\":\"%s\"}\n", result, mode);
        AAudioStream_close(stream);
        AAudioStreamBuilder_delete(builder);
        return 3;
    }

    // Wait for the WALL time, then read the counters: the ratio between frames
    // written and wall time is the clock question, and it cannot be answered by
    // waiting for a number of frames (a stuck stream would wait forever).
    struct timespec nap = {.tv_sec = seconds, .tv_nsec = 0};
    nanosleep(&nap, NULL);
    const double elapsed = now_s() - t0;

    AAudioStream_requestStop(stream);
    const int32_t xruns = AAudioStream_getXRunCount(stream);
    const unsigned long long frames = atomic_load(&g_frames);
    const unsigned long long callbacks = atomic_load(&g_callbacks);
    const int32_t performance_mode = AAudioStream_getPerformanceMode(stream);

    AAudioStream_close(stream);
    AAudioStreamBuilder_delete(builder);

    const double expected = (double)got_rate * elapsed;
    const double ticks = expected > 0 ? (double)frames / expected : 0.0;
    // The prefill: the first callback is handed the whole buffer at once, so the
    // steady ratio is what says whether the HAL consumes at real time.
    const double steady_frames = (double)frames - (double)buffer_frames;
    const double ticks_steady = expected > 0 && steady_frames > 0 ? steady_frames / expected : 0.0;

    printf(
        "{\"opened\":true,\"mode\":\"%s\",\"rate\":%d,\"channels\":%d,"
        "\"burst_frames\":%d,\"buffer_frames\":%d,\"callbacks\":%llu,\"frames\":%llu,"
        "\"elapsed_s\":%.3f,\"ticks\":%.6f,\"ticks_steady\":%.6f,\"xruns\":%d,"
        "\"performance_mode\":%d}\n",
        mode, got_rate, channels, burst, buffer_frames, callbacks, frames, elapsed, ticks,
        ticks_steady, xruns, performance_mode);
    return 0;
}
