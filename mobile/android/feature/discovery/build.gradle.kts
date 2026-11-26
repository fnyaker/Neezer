// :feature/discovery — the Nouveautés module (docs/agents/mobile.md, lot MB14).
//
// One Gradle module per feature lot, disjoint files: a lot fills ITS module and
// edits no other. MB0 created the module, its stub entry composables and the
// signatures the shell and the other feature modules already call; MB14 keeps
// every one of them.

plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
    // T3: verifying goldens (see :testkit).
    id("io.github.takahirom.roborazzi")
}

android {
    namespace = "fr.neezer.mobile.feature.discovery"
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

    // `UseTomlInstead` is answered in the dependency block below: the catalog is
    // created once by MB0 and shared by every lot (worker rule 3), so the four
    // coordinates only this module's JVM tests need are pinned there.
    lint {
        disable += "UseTomlInstead"
    }

    testOptions {
        unitTests {
            // Robolectric renders this lot's screenshots: it needs the merged
            // manifest and the resources of the module (fonts, values).
            isIncludeAndroidResources = true
        }
        // A failing assertion must NAME its two values in the CI log: Gradle's
        // default prints the class and the line only, which cost this lot a
        // whole CI cycle (the six `ExplorerModelTest` failures below).
        unitTests.all {
            it.testLogging {
                exceptionFormat = org.gradle.api.tasks.testing.logging.TestExceptionFormat.FULL
            }
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
    // A row's ⋯ opens MB10's ONE context menu (`TrackActions.open`).
    implementation(project(":feature:trackactions"))
    // `FileProvider`: the Récap's poster is a content:// Uri handed to the
    // share sheet (the provider `:feature:setup` declares, `cache/shared/`).
    implementation(libs.androidx.core.ktx)
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.foundation)
    implementation(libs.kotlinx.coroutines.android)

    // --- JVM tests -----------------------------------------------------------
    // The models are plain Kotlin over their ports (no JNA, no `.so`); the
    // screenshot test renders the REAL screens on Robolectric's native graphics
    // backend — Skia on the host, no emulator, no window on the person's
    // display (worker rule 7), the recipe MB5 measured.
    testImplementation(libs.junit)
    testImplementation(project(":testkit"))
    testImplementation(libs.roborazzi)
    // The models' coroutines run on a VIRTUAL clock (`kotlinx-coroutines-test`),
    // never a `sleep`: `advanceUntilIdle` opens the ONE event a test waits for.
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.11.0")
    testImplementation("org.robolectric:robolectric:4.18")
    testImplementation("androidx.test:core:1.7.0")
    testImplementation("androidx.test.ext:junit:1.3.0")
    testImplementation("androidx.compose.ui:ui-test-junit4")
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
