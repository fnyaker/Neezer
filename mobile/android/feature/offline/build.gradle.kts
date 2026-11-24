// :feature/offline — the Hors ligne module (docs/agents/mobile.md, lot MB15).
//
// One Gradle module per feature lot, disjoint files: a lot fills ITS module and
// edits no other. MB0 created the module, its stub entry composable and the
// signatures the shell and the other feature modules already call; MB15 filled
// the body and added the two coordinates it needs (WorkManager, and the JVM test
// stack the screenshot test renders with).

plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "fr.neezer.mobile.feature.offline"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        minSdk = libs.versions.minSdk.get().toInt()
        // The test APK is an APP on the device: below the device's own targetSdk
        // Android paints its "built for an older version" dialog over the very
        // screen a test waits for (MB7's measurement).
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
    // module's JVM tests need are pinned here.
    lint {
        disable += "UseTomlInstead"
    }

    testOptions {
        unitTests {
            // Robolectric renders the screenshots: it needs the merged manifest
            // and the module's resources (the three fonts, from :designsystem).
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
    // La demande de `POST_NOTIFICATIONS` au premier téléchargement (M13) :
    // `rememberLauncherForActivityResult` vit dans activity-compose, que ce
    // module n'avait pas encore (le catalogue le déclare, MB0 l'a créé).
    implementation(libs.androidx.activity.compose)
    // Below API 34 the queue needs the process kept alive: a `dataSync`
    // foreground worker (docs/agents/mobile.md, "Background downloads"). On API
    // 34+ the same queue runs as a user-initiated data transfer JOB, which is
    // platform-only and needs no dependency.
    implementation(libs.androidx.work.runtime.ktx)

    // --- JVM tests -----------------------------------------------------------
    // `DownloadsModelTest` and `TransferPolicyTest` are plain Kotlin over the
    // lot's ports: no JNA, no `.so`, no device. The screenshot test needs real
    // measure/layout and real pixels, so it runs on Robolectric with its NATIVE
    // graphics backend (Skia on the host) — the recipe MB1 measured and MB5,
    // MB7 and MB10 hold.
    testImplementation(libs.junit)
    testImplementation("org.robolectric:robolectric:4.18")
    testImplementation("androidx.test:core:1.7.0")
    testImplementation("androidx.test.ext:junit:1.3.0")
    testImplementation("androidx.compose.ui:ui-test-junit4")
    debugImplementation("androidx.compose.ui:ui-test-manifest")

    // --- the emulator run (the acceptance's own screenshot) ------------------
    // A Robolectric render proves the layout on the JVM; the lot's acceptance
    // also asks for the REAL screen on a headless emulator, captured from the
    // test APK (no `:app` manifest change, no exported activity, no window on
    // the person's display — worker rule 7).
    androidTestImplementation(libs.junit)
    androidTestImplementation(libs.androidx.test.core)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.rules)
    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(platform(libs.androidx.compose.bom))
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)
}

// The screenshot test writes the PNGs the lot commits under
// `docs/agents/screenshots/mobile/MB15/` (the acceptance asks for them there).
// Absolute paths, handed to the test: no test carries `../../..` arithmetic.
val repoRoot = rootProject.layout.projectDirectory.dir("../..")

tasks.withType<Test>().configureEach {
    systemProperty("nz.repoRoot", repoRoot.asFile.absolutePath)
    systemProperty("nz.screenshots", repoRoot.dir("docs/agents/screenshots/mobile/MB15").asFile.absolutePath)
}
