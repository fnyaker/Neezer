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

    // `CrashRecordTest` (M14) lit et écrit un vrai fichier dans `context.filesDir` :
    // Robolectric lui donne le manifeste fusionné et les ressources du module,
    // comme les autres modules qui rendent ou touchent un `Context` sur la JVM.
    testOptions {
        unitTests {
            isIncludeAndroidResources = true
        }
    }

    // The JVM-test pin below is a literal, as in every feature module: the catalog
    // belongs to MB0 (worker rule 3), and a test-only coordinate is not worth a
    // shared edit.
    lint {
        disable += "UseTomlInstead"
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
// the release library for the ABIs of `NZ_ABIS` (all three by default).
val rustDir = rootProject.layout.projectDirectory.dir("../rust")
val skipRust = providers.gradleProperty("skipRust").isPresent

val buildRust = tasks.register<Exec>("buildRust") {
    group = "build"
    description = "Builds libnz_ffi.so for the ABIs of NZ_ABIS (default all three: arm64-v8a, armeabi-v7a, x86_64) and generates the Kotlin bindings"
    workingDir = rustDir.asFile
    commandLine("./build-android.sh")
    // Re-run when the Rust sources or the workspace's own manifests change —
    // and when the ABI SET changes: `jniLibs/` then holds fewer (or other)
    // libraries, and an up-to-date check that ignored `NZ_ABIS` would keep the
    // previous run's set and pack it into the APK.
    inputs.property("abis", providers.environmentVariable("NZ_ABIS").orElse("all"))
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

/**
 * `rustls-platform-verifier-android`'s version, read from `mobile/rust/Cargo.lock`
 * at configuration time (a `ValueSource`, so the Gradle configuration cache still
 * works). The AAR and the Rust crate must be the same version: the crate's JNI
 * calls name the component's methods, and a mismatch is a handshake that fails
 * with "Expect rustls-platform-verifier to be initialized".
 */
abstract class RustlsVersion : ValueSource<String, RustlsVersion.Params> {
    interface Params : ValueSourceParameters {
        val lockFile: RegularFileProperty
    }

    companion object {
        const val CRATE_NAME = "rustls-platform-verifier-android"
    }

    override fun obtain(): String {
        val lock = parameters.lockFile.get().asFile
        val lines = lock.readLines()
        val index = lines.indexOfFirst { it.trim() == "name = \"$CRATE_NAME\"" }
        check(index >= 0) { "$CRATE_NAME not found in $lock" }
        val versionLine = lines.drop(index + 1)
            .firstOrNull { it.trimStart().startsWith("version = ") }
            ?: error("$CRATE_NAME has no version line in $lock")
        return versionLine.substringAfter('"').substringBefore('"')
    }
}

val rustlsPlatformVerifierVersion: String = providers.of(RustlsVersion::class.java) {
    parameters.lockFile.set(rustDir.file("Cargo.lock"))
}.get()

dependencies {
    // The generated code is JNA-based, and the AAR (not the jar) carries the
    // Android natives: a `jna.jar` on Android dies on its first call.
    implementation(libs.jna) { artifact { type = "aar" } }
    // The generated support code uses coroutines for every `suspend` mapping.
    api(libs.kotlinx.coroutines.core)
    api(libs.kotlinx.coroutines.android)
    // `androidx.annotation.RequiresApi` is referenced by the generated cleaner.
    api(libs.androidx.annotation)

    // --- MB2: the process lifecycle and the cover loader ---------------------
    // `ProcessLifecycleOwner` is what tells the core somebody is looking (rule
    // 1's prefetch gate).
    implementation(libs.androidx.lifecycle.process)
    // Coil: the views decode Rust's file; the singleton lives in `coil`, the
    // fetcher/keyer API in `coil-core` (a transitive `api` of it).
    implementation("io.coil-kt.coil3:coil:${libs.versions.coil.get()}")
    // `AsyncImage` : MB2 n'avait que le singleton et le fetcher ; le seam
    // `NzCoverLoader` du design system a besoin du composable (M11).
    implementation(libs.coil.compose)
    // The Android half of `rustls-platform-verifier`: the JVM component its JNI
    // calls into, at the version the Rust crate locked.
    implementation("org.rustls:rustls-platform-verifier:$rustlsPlatformVerifierVersion")

    // The JVM tests (`screenFlow`, the error register): pure policy, no device.
    testImplementation(libs.junit)
    // `CrashRecordTest` (M14): the storage writes a real file through a real
    // `Context.filesDir`, which android.jar's stubs cannot give. Literal pin, like
    // every module's JVM tests (the catalog belongs to MB0).
    testImplementation("org.robolectric:robolectric:4.18")
}
