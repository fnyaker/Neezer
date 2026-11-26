// :feature/nowplaying — the Lecture en cours module (docs/agents/mobile.md, lot MB11).
//
// One Gradle module per feature lot, disjoint files: a lot fills ITS module and
// edits no other. MB0 created the module, its stub entry composable and the
// signatures the shell and the other feature modules already call.
//
// MB11 added, in this file only (the catalog is MB0's, worker rule 3):
//  - `androidx.core` for the `FileProvider` the lyric card shares through;
//  - `lifecycle-runtime-compose` for `LocalLifecycleOwner.currentStateAsState`,
//    which is what stops the position reads when the screen goes away;
//  - `:feature:trackactions`, because the player's ⋯ and a queue row open MB10's
//    ONE context menu (`TrackActions.open`), never a second one;
//  - the JVM test coordinates its screenshot/model suites need (the recipe MB1
//    measured, MB5/MB7/MB10 hold) and the two `nz.*` system properties that tell
//    the screenshot test where the repository and the lot's images are.

plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "fr.neezer.mobile.feature.nowplaying"
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
    // module's tests need are pinned here.
    lint {
        disable += "UseTomlInstead"
    }

    testOptions {
        unitTests {
            // Robolectric renders the screenshots and the lyric card: it needs
            // the merged manifest and the module's resources (the fonts come from
            // :designsystem, the shared-cards XML from this module).
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
    implementation(project(":feature:trackactions"))
    implementation(project(":feature:remote"))
    implementation(project(":feature:party"))
    implementation(project(":feature:viz"))
    implementation(project(":feature:worlds"))
    implementation(libs.androidx.core.ktx)
    // `androidx.activity.compose` — P2's `BackHandler`: the system back over the
    // player closes the sheet by the SAME path as the chevron and the drag, so it
    // animates out from where the sheet is instead of popping a screen behind it.
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.foundation)
    implementation(libs.kotlinx.coroutines.android)

    // --- JVM tests -----------------------------------------------------------
    // `NowPlayingModelTest`, `LyricsModelTest` and `AboutModelTest` are plain
    // Kotlin over the lot's ports. The screenshot test needs real measure/layout
    // and real pixels, so it runs on Robolectric with its NATIVE graphics
    // backend (Skia on the host).
    testImplementation(libs.junit)
    testImplementation("org.robolectric:robolectric:4.18")
    testImplementation("androidx.test:core:1.7.0")
    testImplementation("androidx.test.ext:junit:1.3.0")
    testImplementation("androidx.compose.ui:ui-test-junit4")
    testImplementation(libs.kotlinx.coroutines.core)
    debugImplementation("androidx.compose.ui:ui-test-manifest")

    // --- the device test (the acceptance's own run) --------------------------
    androidTestImplementation(libs.junit)
    androidTestImplementation(libs.androidx.test.core)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.rules)
    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(platform(libs.androidx.compose.bom))
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)
    // `UiDevice.sleep()` is the DEVICE's own power key: the "screen off" half of
    // the lot's acceptance is that event, not a simulated state.
    androidTestImplementation(libs.androidx.test.uiautomator)
}

// The screenshot test writes the PNGs the lot commits under
// `docs/agents/screenshots/mobile/MB11/` (the acceptance asks for them there).
// Absolute paths, handed to the test: no test carries `../../..` arithmetic.
val repoRoot = rootProject.layout.projectDirectory.dir("../..")

tasks.withType<Test>().configureEach {
    systemProperty("nz.repoRoot", repoRoot.asFile.absolutePath)
    systemProperty("nz.screenshots", repoRoot.dir("docs/agents/screenshots/mobile/MB11").asFile.absolutePath)
}
