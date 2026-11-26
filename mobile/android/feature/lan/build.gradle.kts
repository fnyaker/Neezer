// :feature/lan — the LAN exchange's screens and Android half
// (docs/agents/lan-nodes.md, lot LN14).
//
// One Gradle module per feature lot, disjoint files: a lot fills ITS module and
// edits no other. LN0 created the module, its two entry composables and the two
// routes in `NavGraph.kt`, so this lot fills the bodies and never edits `:app`.
//
// What this lot added to the module's own build file (worker rule 3: a module's
// manifest, provider and dependencies belong to the module):
//
//   * `:feature:offline` — the downloads worker, read through MB15's own public
//     `OfflineHost`: the battery rule needs to know whether a queue is running,
//     and a second copy of that state would be a second truth.
//   * `androidx.lifecycle.process` — `ProcessLifecycleOwner`, the ONE signal of
//     "the app is in front" (rule 1 on the radio).
//   * `androidx.activity.compose` — the local-network permission is asked when
//     the person turns the exchange on, from the settings page itself.
//   * the JVM test stack (Robolectric screenshots) and the instrumented stack
//     (the emulator run, LN14's acceptance).

plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "fr.neezer.mobile.feature.lan"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        minSdk = libs.versions.minSdk.get().toInt()
        // The test APK is an APP on the device: below the device's own targetSdk
        // Android paints its "built for an older version" dialog over the very
        // screen a test waits for (MB7's measurement).
        targetSdk = libs.versions.targetSdk.get().toInt()
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
        buildConfig = false
    }

    testOptions {
        unitTests {
            // Robolectric renders the screenshots: it needs the merged manifest
            // (the provider above) and the module's resources.
            isIncludeAndroidResources = true
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

composeCompiler {
    stabilityConfigurationFiles.add(rootProject.layout.projectDirectory.file("compose-stability.conf"))
}

dependencies {
    implementation(project(":bridge"))
    implementation(project(":designsystem"))
    // MB15's own downloads model: the battery rule asks it whether a queue runs.
    implementation(project(":feature:offline"))
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.foundation)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.androidx.lifecycle.process)
    implementation(libs.androidx.activity.compose)

    // --- JVM tests -----------------------------------------------------------
    // `LanModelTest`, `PeerSetsModelTest`, `LanCopyTest`, `LanActivityTest` are
    // plain Kotlin over the module's ports: no JNA, no `.so`, no device. The
    // screenshots need real measure/layout and real pixels, so they run on
    // Robolectric with its NATIVE graphics backend (MB1's recipe).
    testImplementation(libs.junit)
    testImplementation("org.robolectric:robolectric:4.18")
    testImplementation("androidx.test:core:1.7.0")
    testImplementation("androidx.test.ext:junit:1.3.0")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.11.0")
    testImplementation(libs.androidx.compose.ui.test.junit4)
    debugImplementation("androidx.compose.ui:ui-test-manifest")

    // --- the emulator run (the acceptance's own measures) --------------------
    // NsdManager, the multicast lock and the semantics tree only exist on a
    // device: `LanNsdTest` and `LanCaptureTest` run on the headless emulator
    // (mobile/tools/emu.sh, `-no-window -no-audio`).
    androidTestImplementation(libs.junit)
    androidTestImplementation(libs.androidx.test.core)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.rules)
    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(platform(libs.androidx.compose.bom))
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)
}

// The screenshot test writes the PNGs the lot commits under
// `docs/agents/screenshots/lan/LN14/` (the acceptance asks for them there).
val repoRoot = rootProject.layout.projectDirectory.dir("../..")

tasks.withType<Test>().configureEach {
    systemProperty("nz.repoRoot", repoRoot.asFile.absolutePath)
}
