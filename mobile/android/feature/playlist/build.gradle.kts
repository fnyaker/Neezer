// :feature/playlist — the Playlist module (docs/agents/mobile.md, lot MB9).
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
    namespace = "fr.neezer.mobile.feature.playlist"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        minSdk = libs.versions.minSdk.get().toInt()
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
            // Robolectric renders this screen's screenshots: it needs the merged
            // manifest and the resources of the module (fonts, values).
            isIncludeAndroidResources = true
        }
    }

    // `UseTomlInstead` is answered by the catalog MB0 pinned (worker rule 3).
    lint {
        disable += "UseTomlInstead"
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
    implementation(project(":feature:offline"))
    implementation(project(":feature:trackactions"))

    // --- JVM tests -----------------------------------------------------------
    // `PlaylistModelTest` is plain Kotlin over the screen's ports (a fake 4 000-row
    // source, a fake player, a fake sharer) on a VIRTUAL clock — never a sleep
    // (worker rule 6). The screenshot test needs real measure/layout and real
    // pixels, so it runs on Robolectric with its NATIVE graphics backend (Skia on
    // the host): no emulator, no window on the person's display (worker rule 7),
    // the recipe MB1 measured.
    testImplementation(libs.junit)
    testImplementation(project(":testkit"))
    testImplementation(libs.roborazzi)
    testImplementation("org.robolectric:robolectric:4.18")
    testImplementation("androidx.test:core:1.7.0")
    testImplementation("androidx.test.ext:junit:1.3.0")
    testImplementation("androidx.compose.ui:ui-test-junit4")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.11.0")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}

// T3: the goldens live in `mobile/android/screenshots`; `nz.repoRoot` stays
// for the tests that read the SPA's own files.
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
