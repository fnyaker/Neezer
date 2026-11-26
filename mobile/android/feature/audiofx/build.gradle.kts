// :feature/audiofx — the Effets audio module (docs/agents/mobile.md, lot MB12).
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
    namespace = "fr.neezer.mobile.feature.audiofx"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        minSdk = libs.versions.minSdk.get().toInt()
        // The device capture runs as an instrumented test on the emulator.
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

    // The JVM-test pins this module needs are literal: the catalog belongs to
    // MB0 (worker rule 3), and a test-only coordinate is not worth a shared edit.
    lint {
        disable += "UseTomlInstead"
    }

    testOptions {
        unitTests {
            // Robolectric renders the screenshots: it needs the merged manifest
            // and this module's resources.
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
    // `AudioFxModelTest` is plain Kotlin over the screen's port: no JNA, no
    // `.so`, no device. The screenshots need real measure/layout and real
    // pixels, so they run on Robolectric's NATIVE graphics backend (the recipe
    // MB1 measured and MB5/MB6/MB7/MB16 reused): no emulator, no window on the
    // person's display (worker rule 7).
    testImplementation(libs.junit)
    testImplementation("org.robolectric:robolectric:4.18")
    testImplementation("androidx.test:core:1.7.0")
    testImplementation("androidx.test.ext:junit:1.3.0")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.11.0")
    testImplementation(libs.androidx.compose.ui.test.junit4)
    testImplementation("androidx.activity:activity-compose:1.11.0")
    debugImplementation("androidx.compose.ui:ui-test-manifest")

    // --- device test: the real card on the emulator, for the capture ----------
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test:core:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
    androidTestImplementation(platform(libs.androidx.compose.bom))
    androidTestImplementation("androidx.activity:activity-compose:1.11.0")
}

// T3: the goldens live in `mobile/android/screenshots`; `nz.repoRoot` stays
// for the tests that read the SPA's own files.
val repoRoot = rootProject.layout.projectDirectory.dir("../..")

tasks.withType<Test>().configureEach {
    systemProperty("nz.repoRoot", repoRoot.asFile.absolutePath)
}
