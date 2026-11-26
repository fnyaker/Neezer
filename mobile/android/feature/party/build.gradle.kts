// :feature/party — the Soirée d'écoute module (docs/agents/mobile.md, lot MB20).
//
// One Gradle module per feature lot, disjoint files: a lot fills ITS module and
// edits no other. MB0 created the module, its stub entry composable and the
// signatures the shell and the other feature modules already call.
//
// MB20 added, in this file only (the catalog is MB0's, worker rule 3):
//  - `lifecycle-runtime-compose` for `collectAsStateWithLifecycle` and
//    `LifecycleResumeEffect` (the guest's playhead stops when the screen does);
//  - ZXing's core, the link's QR code (`PartyQr.kt`; the SPA uses
//    `qrcode-generator`, and the matrix is the same standard);
//  - the JVM test coordinates the model and screenshot suites need (the recipe
//    MB1 measured and MB5/MB7/MB10/MB11/MB13 hold).

plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "fr.neezer.mobile.feature.party"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        minSdk = libs.versions.minSdk.get().toInt()
        // The instrumented test APK is an APP on the device: below the device's
        // own targetSdk Android paints its "built for an older version" dialog
        // over the very screen a test waits for (MB7's measurement).
        targetSdk = libs.versions.targetSdk.get().toInt()
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

    // `UseTomlInstead` is answered in the dependency block: the catalog is shared
    // and created once by MB0 (worker rule 3), so the coordinates only this
    // module's own tests need are pinned here.
    lint {
        disable += "UseTomlInstead"
    }

    testOptions {
        unitTests {
            // Robolectric renders the screenshots: it needs the merged manifest
            // and the module's resources (the fonts come from :designsystem).
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
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.foundation)
    implementation(libs.kotlinx.coroutines.android)
    // `PARTY_QR`: the link as a QR code, drawn on Compose's canvas.
    implementation("com.google.zxing:core:3.5.3")

    // --- JVM tests -----------------------------------------------------------
    testImplementation(libs.junit)
    testImplementation("org.robolectric:robolectric:4.18")
    testImplementation("androidx.test:core:1.7.0")
    testImplementation("androidx.test.ext:junit:1.3.0")
    testImplementation("androidx.compose.ui:ui-test-junit4")
    testImplementation(libs.kotlinx.coroutines.core)
    // A VIRTUAL clock for the debounces (`PartyModelTest`), never a sleep.
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.11.0")
    debugImplementation("androidx.compose.ui:ui-test-manifest")

    // --- the device test (the acceptance's own run) --------------------------
    androidTestImplementation(libs.junit)
    androidTestImplementation(libs.androidx.test.core)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.rules)
    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(libs.androidx.test.uiautomator)
    androidTestImplementation(platform(libs.androidx.compose.bom))
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)
}

// T3: the goldens live in `mobile/android/screenshots`; `nz.repoRoot` stays
// for the tests that read the SPA's own files.
// Absolute paths, handed to the test: no test carries `../../..` arithmetic.
val repoRoot = rootProject.layout.projectDirectory.dir("../..")

tasks.withType<Test>().configureEach {
    systemProperty("nz.repoRoot", repoRoot.asFile.absolutePath)
}
