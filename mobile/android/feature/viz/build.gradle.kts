// :feature/viz — the Visualiseur module (docs/agents/mobile.md, lot MB21).
//
// One Gradle module per feature lot, disjoint files: a lot fills ITS module and
// edits no other. MB0 created the module, its stub entry composable and the
// signatures the shell and the other feature modules already call.
//
// MB21 added, in THIS file only (the catalog is MB0's, worker rule 3):
//  - `:feature:entity` nothing, `:bridge`/`:designsystem` were already here;
//  - the JVM test coordinates its model and screenshot suites need (the recipe
//    MB1 measured, MB5/MB7/MB10/MB11 hold) and the two `nz.*` system properties
//    that tell the screenshot test where the repository and the lot's images are;
//  - `UseTomlInstead` disabled with that reason, as MB11 did.

plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "fr.neezer.mobile.feature.viz"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        minSdk = libs.versions.minSdk.get().toInt()
        // The instrumented test APK is an APP on the device: below the device's
        // own targetSdk Android paints its "built for an older version" dialog
        // over the very screen a test waits for (MB7's measurement).
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

    lint {
        // The catalog is shared and created once by MB0; the coordinates only
        // this module's tests need are pinned below, with their versions.
        disable += "UseTomlInstead"
    }

    testOptions {
        unitTests {
            // Robolectric renders the screenshots: it needs the merged manifest
            // and the module's resources (the fonts come from :designsystem).
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
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.foundation)
    implementation(libs.kotlinx.coroutines.android)

    // --- JVM tests -----------------------------------------------------------
    // `VizModelTest` is plain Kotlin over the lot's ports; the screenshot suite
    // needs real measure/layout and real pixels, so it runs on Robolectric with
    // its NATIVE graphics backend (Skia on the host) — the recipe MB1 measured.
    testImplementation(libs.junit)
    testImplementation("org.robolectric:robolectric:4.18")
    testImplementation("androidx.test:core:1.7.0")
    testImplementation("androidx.test.ext:junit:1.3.0")
    testImplementation("androidx.compose.ui:ui-test-junit4")
    testImplementation(libs.kotlinx.coroutines.core)
    debugImplementation("androidx.compose.ui:ui-test-manifest")

    // --- the device test (the acceptance's own run) --------------------------
    // `VizDeviceTest` opens the REAL screen on the emulator, reads the frame
    // times of its paint loop (`FrameMetrics`) and takes the device's own
    // screenshot — the two numbers an acceptance about "p95 ≤ 16.7 ms" and "a
    // headless-emulator capture of the real screen" needs.
    androidTestImplementation(libs.junit)
    androidTestImplementation(libs.androidx.test.core)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.rules)
    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(platform(libs.androidx.compose.bom))
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)
    androidTestImplementation(libs.androidx.test.uiautomator)
}

// The screenshot suite writes the PNGs the lot commits under
// `docs/agents/screenshots/mobile/MB21/` (the acceptance asks for them there).
// Absolute paths, handed to the test: no test carries `../../..` arithmetic.
val repoRoot = rootProject.layout.projectDirectory.dir("../..")

tasks.withType<Test>().configureEach {
    systemProperty("nz.repoRoot", repoRoot.asFile.absolutePath)
    systemProperty("nz.screenshots", repoRoot.dir("docs/agents/screenshots/mobile/MB21").asFile.absolutePath)
}
