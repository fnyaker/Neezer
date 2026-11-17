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
