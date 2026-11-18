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
    // The models' coroutines run on a VIRTUAL clock (`kotlinx-coroutines-test`),
    // never a `sleep`: `advanceUntilIdle` opens the ONE event a test waits for.
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.11.0")
    testImplementation("org.robolectric:robolectric:4.18")
    testImplementation("androidx.test:core:1.7.0")
    testImplementation("androidx.test.ext:junit:1.3.0")
    testImplementation("androidx.compose.ui:ui-test-junit4")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}

// The screenshot test writes the PNGs the lot commits under
// `docs/agents/screenshots/mobile/MB14/` (the acceptance asks for them there).
val repoRoot = rootProject.layout.projectDirectory.dir("../..")

tasks.withType<Test>().configureEach {
    systemProperty("nz.repoRoot", repoRoot.asFile.absolutePath)
    systemProperty("nz.screenshots", repoRoot.dir("docs/agents/screenshots/mobile/MB14").asFile.absolutePath)
}
