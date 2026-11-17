// :bridge — the generated UniFFI Kotlin, the Flow adapters and the `NzCore`
// holder (docs/agents/mobile.md, ":bridge"). Owned by MB2 after MB0.
//
// The native library is NOT built by a third-party Gradle plugin: an `Exec` task
// runs the one script that documents the whole thing (host debug build →
// bindgen → cargo ndk for three ABIs). A lot that only touches Kotlin passes
// `-PskipRust` and reuses the artifacts a previous build left.

plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
}

android {
    namespace = "fr.neezer.mobile.bridge"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        minSdk = libs.versions.minSdk.get().toInt()
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        // The generated bindings are Kotlin AND the JNI exports are looked up
        // through JNA, so this module keeps no BuildConfig.
        buildConfig = false
    }

    sourceSets["main"].java.srcDir(layout.buildDirectory.dir("generated/uniffi"))
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

// --- the native side -------------------------------------------------------
// `mobile/rust/build-android.sh` is the one documented path (docs/agents/
// mobile.md, MB0 "Contract"): the host DEBUG library feeds `uniffi-bindgen`
// (a stripped release `.so` has no metadata), then `cargo ndk` cross-compiles
// the release library for the three ABIs the APK ships.
val rustDir = rootProject.layout.projectDirectory.dir("../rust")
val skipRust = providers.gradleProperty("skipRust").isPresent

val buildRust = tasks.register<Exec>("buildRust") {
    group = "build"
    description = "Builds libnz_ffi.so for arm64-v8a, armeabi-v7a and x86_64 and generates the Kotlin bindings"
    workingDir = rustDir.asFile
    commandLine("./build-android.sh")
    // Re-run when the Rust sources or the workspace's own manifests change.
    inputs.files(
        rustDir.file("Cargo.toml"),
        rustDir.file("Cargo.lock"),
        rustDir.file("uniffi.toml"),
        rustDir.file("build-android.sh"),
    )
    inputs.dir(rustDir.dir("crates"))
    inputs.dir(rustDir.dir("bindgen"))
    outputs.dir(layout.buildDirectory.dir("generated/uniffi"))
    outputs.dir(project.layout.projectDirectory.dir("src/main/jniLibs"))
}

// `-PskipRust` is for the JVM-only CI steps (`testDebugUnitTest`, `lintDebug`,
// `assembleRelease`) that reuse an APK job's artifacts: without it every Gradle
// invocation would cross-compile the whole stack again.
//
// `lint*` is wired by NAME rather than by AGP's own task class: that class moved
// between AGP minor versions, and a build script naming it stops configuring the
// day it moves (`LintBaseTask` is already gone in AGP 8.13).
tasks.configureEach {
    if (!skipRust && (name == "preBuild" || name.startsWith("lint"))) {
        dependsOn(buildRust)
    }
}

dependencies {
    // The generated code is JNA-based, and the AAR (not the jar) carries the
    // Android natives: a `jna.jar` on Android dies on its first call.
    implementation(libs.jna) { artifact { type = "aar" } }
    // The generated support code uses coroutines for every `suspend` mapping.
    api(libs.kotlinx.coroutines.core)
    api(libs.kotlinx.coroutines.android)
    // `androidx.annotation.RequiresApi` is referenced by the generated cleaner.
    api(libs.androidx.annotation)
}
