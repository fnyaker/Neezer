// :feature/genres — the genre studio module (docs/agents/mobile.md, lot MB18).
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
    namespace = "fr.neezer.mobile.feature.genres"
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

    // `GenresModelTest` is plain Kotlin over the screen's port, the tree's window
    // is measured on a fake 1 000-row source, and the screenshots render on
    // Robolectric: it needs the merged manifest and this module's resources for a
    // real layout and real text. Robolectric is the ONE coordinate the version
    // catalog does not carry (the catalog is MB0's file, worker rule 3), so it is
    // pinned here at the version MB16 measured — the same pin every feature module
    // uses, and the reason `UseTomlInstead` is off for this module only.
    lint {
        disable += "UseTomlInstead"
    }

    testOptions {
        unitTests {
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
    testImplementation(libs.junit)
    testImplementation(project(":testkit"))
    testImplementation(libs.roborazzi)
    testImplementation("org.robolectric:robolectric:4.18")
    testImplementation(libs.androidx.test.core)
    testImplementation(libs.androidx.test.ext.junit)
    // The version comes from the catalog (`coroutines`), never from a literal: a
    // `coroutines-test` that disagreed with the runtime would fail in a way only
    // a device shows.
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:${libs.versions.coroutines.get()}")
    testImplementation(libs.androidx.compose.ui.test.junit4)
    debugImplementation(libs.androidx.compose.ui.test.manifest)
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
