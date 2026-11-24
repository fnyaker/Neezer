// :feature/settings — the Réglages module (docs/agents/mobile.md, lot MB16).
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
    namespace = "fr.neezer.mobile.feature.settings"
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

    // The JVM-test pins this module needs are literal: the catalog belongs to
    // MB0 (worker rule 3), and a test-only coordinate is not worth a shared edit.
    lint {
        disable += "UseTomlInstead"
    }

    testOptions {
        unitTests {
            // Robolectric renders the screenshots: it needs the merged manifest
            // and this module's resources (fonts, values).
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
    // MB12's own effects cards and MB17's own pages: hosted, never copied.
    implementation(project(":feature:audiofx"))
    implementation(project(":feature:admin"))
    implementation(project(":feature:offline"))
    // Assistance hosts the app's own update card (MB4's `Updater`, the native
    // channel): the module that owns the updater exposes it, this one draws it.
    implementation(project(":feature:setup"))
    // LN14's « Appareils à proximité »: MB16's extension point ("one entry to
    // that table plus one branch that calls its own composable"), so the module
    // that owns the screen is a dependency of the module that hosts it.
    implementation(project(":feature:lan"))
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.foundation)
    implementation(libs.kotlinx.coroutines.android)

    // --- JVM tests -----------------------------------------------------------
    // `SettingsModelTest` is plain Kotlin over the screen's port: no JNA, no
    // `.so`, no device. The screenshots need real measure/layout and real
    // pixels, so they run on Robolectric with its NATIVE graphics backend (the
    // recipe MB1 measured and MB5/MB6/MB7 reused): no emulator, no window on the
    // person's display (worker rule 7).
    testImplementation(libs.junit)
    testImplementation("org.robolectric:robolectric:4.18")
    testImplementation("androidx.test:core:1.7.0")
    testImplementation("androidx.test.ext:junit:1.3.0")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.11.0")
    testImplementation(libs.androidx.compose.ui.test.junit4)
    // The theme-switch test renders ANOTHER feature (the home) inside the
    // wrapper, which is what the acceptance asks it to prove.
    testImplementation(project(":feature:home"))
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}

// The screenshot test writes the PNGs the lot commits under
// `docs/agents/screenshots/mobile/MB16/` (the acceptance asks for them there).
val repoRoot = rootProject.layout.projectDirectory.dir("../..")

tasks.withType<Test>().configureEach {
    systemProperty("nz.repoRoot", repoRoot.asFile.absolutePath)
    systemProperty("nz.screenshots", repoRoot.dir("docs/agents/screenshots/mobile/MB16").asFile.absolutePath)
}
