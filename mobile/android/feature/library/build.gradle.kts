// :feature/library — the Bibliothèque module (docs/agents/mobile.md, lot MB8).
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
    namespace = "fr.neezer.mobile.feature.library"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        minSdk = libs.versions.minSdk.get().toInt()
        // The test APK is an APP on the device, and one whose targetSdk is below
        // the device's gets Android's "This app was built for an older version"
        // dialog over the very screen a test waits for (MB7 measured it).
        targetSdk = libs.versions.targetSdk.get().toInt()
        // Without a runner an androidTest module is a JUnit3 one and runs ZERO
        // tests ("Starting 0 tests on emulator-5640"): measured on this lot's
        // first device run, where the class was in the APK all along.
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

    // `UseTomlInstead` is answered below: the catalog is created once by MB0 and
    // shared by every lot (worker rule 3), so the Robolectric coordinates only
    // this module's JVM tests need stay literal here.
    lint {
        disable += "UseTomlInstead"
    }

    testOptions {
        unitTests {
            // Robolectric renders this screen's screenshots: it needs the merged
            // manifest and the module's resources (the three fonts, the values).
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
    // A row's ⋯ opens MB10's ONE context menu: this module declares the stub's
    // module and never copies the menu.
    implementation(project(":feature:trackactions"))
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.foundation)
    implementation(libs.kotlinx.coroutines.android)

    // --- JVM tests -----------------------------------------------------------
    // `LibraryModelTest` and `HistoryModelTest` are plain Kotlin over the
    // screens' ports: no JNA, no `.so`, no device. The screenshot test needs real
    // measure/layout and real pixels, so it runs on Robolectric with its NATIVE
    // graphics backend (Skia on the host) — no emulator, no window on the
    // person's display (worker rule 7), the recipe MB1 measured and MB5/MB7 used.
    testImplementation(libs.junit)
    testImplementation(project(":testkit"))
    testImplementation(libs.roborazzi)
    testImplementation("org.robolectric:robolectric:4.18")
    testImplementation("androidx.test:core:1.7.0")
    testImplementation("androidx.test.ext:junit:1.3.0")
    testImplementation("androidx.compose.ui:ui-test-junit4")
    debugImplementation("androidx.compose.ui:ui-test-manifest")

    // --- the device test (one file, the acceptance's own run) ----------------
    androidTestImplementation(libs.junit)
    androidTestImplementation(libs.androidx.test.core)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.rules)
    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(platform(libs.androidx.compose.bom))
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)
    // The fling is driven by uiautomator's own touch injection, exactly as the
    // plan's `:baselineprofile` benches fling (`BenchTarget.fling`).
    androidTestImplementation(libs.androidx.test.uiautomator)
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
