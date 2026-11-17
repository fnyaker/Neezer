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
}
