// :feature/podcasts — the Podcasts module (docs/agents/mobile.md, lot MB13).
//
// One Gradle module per feature lot, disjoint files: a lot fills ITS module and
// edits no other. MB0 created the module, its stub entry composable and the
// signatures the shell and the other feature modules already call.

plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
    // T3: verifying goldens (see :testkit).
    id("io.github.takahirom.roborazzi")
}

android {
    namespace = "fr.neezer.mobile.feature.podcasts"
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

    // `UseTomlInstead` is answered in the dependency block: the catalog is shared
    // and created once by MB0 (worker rule 3), so the coordinates only this
    // module's JVM tests need are pinned here.
    lint {
        disable += "UseTomlInstead"
    }

    testOptions {
        unitTests {
            // Robolectric renders the screenshots: it needs the merged manifest
            // and the module's resources (the three fonts, from :designsystem).
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
    // MB15's DownloadButton, hosted (never copied): a show downloads through it.
    implementation(project(":feature:offline"))
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.foundation)
    implementation(libs.kotlinx.coroutines.android)

    // --- JVM tests -----------------------------------------------------------
    // The models are plain Kotlin over the lot's ports: no JNA, no `.so`, no
    // device. The screenshots need real measure/layout and real pixels, so they
    // run on Robolectric with its NATIVE graphics backend (the recipe MB1
    // measured and MB5/MB7/MB10/MB15 reused): no emulator, no window on the
    // person's display (worker rule 7).
    testImplementation(libs.junit)
    testImplementation(project(":testkit"))
    testImplementation(libs.roborazzi)
    testImplementation("org.robolectric:robolectric:4.18")
    testImplementation("androidx.test:core:1.7.0")
    testImplementation("androidx.test.ext:junit:1.3.0")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.11.0")
    testImplementation(libs.androidx.compose.ui.test.junit4)
    debugImplementation("androidx.compose.ui:ui-test-manifest")

    // --- the emulator run (the acceptance's own screenshot) ------------------
    // A Robolectric render proves the layout on the JVM; the lot's acceptance
    // also asks for the REAL screen on a headless emulator, captured from the
    // test APK (no `:app` manifest change, no exported activity, no window on
    // the person's display — worker rule 7).
    androidTestImplementation(libs.junit)
    androidTestImplementation(libs.androidx.test.core)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.rules)
    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(platform(libs.androidx.compose.bom))
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)
}

// T3: the goldens live in `mobile/android/screenshots`; `nz.repoRoot` stays
// for the tests that read the SPA's own files.
// Absolute paths, handed to the test: no test carries `../../..` arithmetic.
val repoRoot = rootProject.layout.projectDirectory.dir("../..")

// T3: the ONE committed golden root. The rule compares against it by absolute
// path, and `inputs.dir` makes a golden change rerun this module's tests. The
// plugin keeps its OWN output directory (`build/outputs/roborazzi`): its finalize
// task copies the verify input back, and a shared directory raced between the
// modules Gradle runs in parallel (measured: a truncated PNG and an IIOException
// in `:feature:entity`).
val goldenRoot = rootProject.layout.projectDirectory.dir("screenshots")

tasks.withType<Test>().configureEach {
    systemProperty("nz.repoRoot", repoRoot.asFile.absolutePath)
    systemProperty("nz.goldens", goldenRoot.asFile.absolutePath)
    inputs.dir(goldenRoot)
}
