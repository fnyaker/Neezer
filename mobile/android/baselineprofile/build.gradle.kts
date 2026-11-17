// :baselineprofile — the Macrobenchmark module and the Baseline Profile
// generator (docs/agents/mobile.md, MQ1/MQ2). MB0 creates the module so the
// plugin, the target project and the build type exist; MQ1 writes the
// `BaselineProfileRule` generator, MQ2 the whole-journey benchmarks.
//
// A Baseline Profile is what turns the published 324.8 ms → 229.0 ms startup
// sample into the app's own start (docs/agents/mobile.md, "Size and cold
// start"); it is installed by `profileinstaller` in :app on first run.

plugins {
    alias(libs.plugins.android.test)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.baselineprofile)
}

android {
    namespace = "fr.neezer.mobile.baselineprofile"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        minSdk = libs.versions.minSdk.get().toInt()
        targetSdk = libs.versions.targetSdk.get().toInt()
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        // The emulator is where these benches RUN (docs/agents/mobile.md,
        // "Measures": "emulator x86_64, KVM, swiftshader; real phone numbers
        // reported beside"). Macrobenchmark fails a run on an emulator by
        // default, with a paragraph saying emulator numbers do not carry over to
        // a real phone — true, and exactly why every number MQ1 writes is
        // labelled with its emulator and compared run to run on the SAME one,
        // never against the plan's real-device ceilings. Suppressing the error is
        // therefore the deliberate choice, not a shortcut: leaving it on would
        // mean the harness cannot run anywhere in CI at all.
        testInstrumentationRunnerArguments["androidx.benchmark.suppressErrors"] = "EMULATOR"
        // The profile GENERATOR has its own emulator guard, with the same
        // reasoning and the same answer: without this line `BaselineProfileRule`
        // reports the test as skipped, with the message "Skipping test because
        // it's running on emulator and `skipOnEmulator` is enabled" and no
        // profile at all. A profile generated here is good enough to prove the
        // journey and the tooling; the one the release SHIPS is MQ2's, generated
        // over the whole app.
        testInstrumentationRunnerArguments["androidx.benchmark.skipBenchmarksOnEmulator"] = "false"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        buildConfig = false
    }

    // The benchmarks run against :app's `benchmark` build type (a release-shaped
    // build that is not debuggable, which `BenchmarkRule` requires).
    targetProjectPath = ":app"
    @Suppress("UnstableApiUsage")
    experimentalProperties["android.experimental.self-instrumenting"] = true
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

dependencies {
    implementation(libs.androidx.benchmark.macro.junit4)
    implementation(libs.androidx.benchmark.junit4)
    implementation(libs.junit)
}
