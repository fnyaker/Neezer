// :feature/playlist — the Playlist module (docs/agents/mobile.md, lot MB9).
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
    testImplementation("org.robolectric:robolectric:4.18")
    testImplementation("androidx.test:core:1.7.0")
    testImplementation("androidx.test.ext:junit:1.3.0")
    testImplementation("androidx.compose.ui:ui-test-junit4")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.11.0")
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}

// The screenshot test writes the PNGs the lot commits under
// `docs/agents/screenshots/mobile/MB9/` (the acceptance asks for them there).
val repoRoot = rootProject.layout.projectDirectory.dir("../..")

tasks.withType<Test>().configureEach {
    systemProperty("nz.repoRoot", repoRoot.asFile.absolutePath)
    systemProperty("nz.screenshots", repoRoot.dir("docs/agents/screenshots/mobile/MB9").asFile.absolutePath)
}
