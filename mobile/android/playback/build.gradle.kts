// :playback — the media service, the `NzPlayer` facade, audio focus, the locks,
// the API-26 AudioTrack pull (docs/agents/mobile.md, MB3c; `AudioTrackPull.kt` is
// MB3a's).
//
// MB0 creates the module and a service that answers no session yet. The
// notification, the lock screen, headset buttons, Bluetooth and Android Auto come
// from Media3 for free once MB3c fills the facade: this module holds no player
// logic of its own, it mirrors the Rust snapshot.

plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
}

android {
    namespace = "fr.neezer.mobile.playback"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        minSdk = libs.versions.minSdk.get().toInt()

        // MB3c's acceptance is instrumented (a real MediaController, real audio
        // focus, a real screen-off run), so the module carries its own device
        // tests: `:playback:connectedDebugAndroidTest` on a headless emulator.
        // The runner and the pins come from MQ1's `libs.versions.toml` entries;
        // no version is written here.
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    testOptions {
        // The JVM tests are pure Kotlin (window arithmetic, the focus table, the
        // lock rule, the battery flag): no Android framework stubs to shadow.
        unitTests.isIncludeAndroidResources = false
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        buildConfig = false
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

dependencies {
    implementation(project(":bridge"))
    implementation(libs.androidx.media3.session)
    implementation(libs.androidx.media3.common)
    implementation(libs.kotlinx.coroutines.android)

    // The policy classes (PlaybackWindow, QueueEdit, FocusPolicy, LockPolicy,
    // BatteryExemption) run on the JVM: no device, no Robolectric.
    testImplementation(libs.junit)

    // MB3c's instrumented acceptance, on the headless emulator: a MediaController
    // that drives the session with the activity FINISHED, the headset keys with
    // the screen off, the 30-minute screen-off + Doze run, the focus hand-over
    // and the notification capture. `:app`'s own `androidTest` proves the chain
    // for the shell; this module proves the service.
    androidTestImplementation(libs.androidx.test.core)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.rules)
    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(libs.androidx.test.uiautomator)
}
