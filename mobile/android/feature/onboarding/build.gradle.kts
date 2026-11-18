// :feature/onboarding — the Bienvenue module (docs/agents/mobile.md, lot MB5b).
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
    namespace = "fr.neezer.mobile.feature.onboarding"
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
    // created once by MB0 and shared by every lot (worker rule 3), so the
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
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.foundation)
    implementation(libs.kotlinx.coroutines.android)

    // --- JVM tests -----------------------------------------------------------
    // `OnboardingModelTest`, `InviteModelTest` and `OnboardingCopyTest` are plain
    // Kotlin over the screen's ports: no JNA, no `.so`, no device. Every wait is
    // a VIRTUAL clock (`kotlinx-coroutines-test`), never a sleep. The screenshot
    // test needs real measure/layout and real pixels, so it runs on Robolectric
    // with its NATIVE graphics backend (Skia on the host) — no emulator, no
    // window on the person's display (worker rule 7), the recipe MB1 measured.
    testImplementation(libs.junit)
    testImplementation("org.robolectric:robolectric:4.18")
    testImplementation("androidx.test:core:1.7.0")
    testImplementation("androidx.test.ext:junit:1.3.0")
    testImplementation("androidx.compose.ui:ui-test-junit4")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.11.0")
    // `createComposeRule()` hosts its content in an activity the test manifest
    // declares; without it Robolectric finds no window to render.
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}

// The screenshot test writes the PNGs the lot commits under
// `docs/agents/screenshots/mobile/MB5b/` (the acceptance asks for them there).
// Absolute paths, handed to the test: no test carries `../../..` arithmetic.
val repoRoot = rootProject.layout.projectDirectory.dir("../..")

tasks.withType<Test>().configureEach {
    systemProperty("nz.repoRoot", repoRoot.asFile.absolutePath)
    systemProperty("nz.screenshots", repoRoot.dir("docs/agents/screenshots/mobile/MB5b").asFile.absolutePath)
}
