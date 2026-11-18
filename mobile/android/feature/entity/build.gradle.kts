// :feature/entity — the Artiste, album et crédits module (docs/agents/mobile.md, lot MB7).
//
// One Gradle module per feature lot, disjoint files: a lot fills ITS module and
// edits no other. MB0 created the module, its stub entry composable and the
// signatures the shell and the other feature modules already call.

plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "fr.neezer.mobile.feature.entity"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        minSdk = libs.versions.minSdk.get().toInt()
        // The test APK is an APP on the device, and one whose targetSdk is below
        // the device's gets Android's "This app was built for an older version"
        // dialog over the very screen a test is waiting for — measured on the API
        // 35 emulator, where it covered the page and the wait timed out.
        targetSdk = libs.versions.targetSdk.get().toInt()
        // The lot's own instrumented test (a real screen over the real core on the
        // emulator): it needs the runner, exactly as `:app`'s smoke does.
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

    // `UseTomlInstead` is answered in the dependency block below: the catalog is
    // created once by MB0 and shared by every lot (worker rule 3), so the three
    // coordinates only this module's JVM tests need are pinned there.
    lint {
        disable += "UseTomlInstead"
    }

    testOptions {
        unitTests {
            // Robolectric renders this screen's screenshots: it needs the merged
            // manifest and the resources of the module (fonts, values).
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
    // A row's ⋯ opens MB10's ONE context menu, and an album's « Télécharger » is
    // MB15's own button: this module declares the stubs' modules and never copies
    // the menu or the download queue.
    implementation(project(":feature:trackactions"))
    implementation(project(":feature:offline"))
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.foundation)
    implementation(libs.kotlinx.coroutines.android)

    // --- JVM tests -----------------------------------------------------------
    // `EntityModelTest` is plain Kotlin over the screens' ports: no JNA, no `.so`,
    // no device. The screenshot test needs real measure/layout and real pixels, so
    // it runs on Robolectric with its NATIVE graphics backend (Skia on the host) —
    // no emulator, no window on the person's display (worker rule 7), the same
    // recipe MB1 measured and MB5 used (`:designsystem`'s build file says why
    // Paparazzi cannot render this tree's Compose).
    testImplementation(libs.junit)
    testImplementation("org.robolectric:robolectric:4.18")
    testImplementation("androidx.test:core:1.7.0")
    testImplementation("androidx.test.ext:junit:1.3.0")
    testImplementation("androidx.compose.ui:ui-test-junit4")
    // `createComposeRule()` hosts its content in an activity the test manifest
    // declares; without it Robolectric finds no window to render.
    debugImplementation("androidx.compose.ui:ui-test-manifest")

    // --- the device test (one file, the acceptance's own run) ----------------
    androidTestImplementation(libs.junit)
    androidTestImplementation(libs.androidx.test.core)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.rules)
    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(platform(libs.androidx.compose.bom))
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)
}

// The screenshot test writes the PNGs the lot commits under
// `docs/agents/screenshots/mobile/MB7/` (the acceptance asks for them there).
// Absolute paths, handed to the test: no test carries `../../..` arithmetic.
val repoRoot = rootProject.layout.projectDirectory.dir("../..")

tasks.withType<Test>().configureEach {
    systemProperty("nz.repoRoot", repoRoot.asFile.absolutePath)
    systemProperty("nz.screenshots", repoRoot.dir("docs/agents/screenshots/mobile/MB7").asFile.absolutePath)
}
