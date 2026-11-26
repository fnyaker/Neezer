// :designsystem — Sillon in Compose (docs/agents/mobile.md, MB1).
//
// Everything a screen lot draws lives here and nowhere else: the tokens of
// `webapp/src/app.css`, the three fonts, the motion table of
// `webapp/src/lib/motion.js` with the eco/reduced guard, the glyphs of
// `webapp/src/components/Icon.svelte` and the components of the MB1 contract.
// A screen lot writes `Sillon.…` and `Nz…`, never a literal colour, size or
// duration (rule 5).

plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
    // T3: verifying goldens. The plugin is what reads
    // `-Proborazzi.test.verify=true`; without it a capture is a silent no-op.
    id("io.github.takahirom.roborazzi")
}

android {
    namespace = "fr.neezer.mobile.designsystem"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        minSdk = libs.versions.minSdk.get().toInt()
        // `RowSourceTest` is the module's ONE instrumented test: a `ListHandle` is
        // a UniFFI object, so the JVM cannot build one, and the bug that test
        // exists for (a row source that called itself) is invisible to a fake.
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

    // `UseTomlInstead` is answered in the dependency block below: the catalog is
    // created once by MB0 and shared by every lot (worker rule 3), so the three
    // coordinates only this module's JVM tests need are pinned here.
    lint {
        disable += "UseTomlInstead"
    }

    testOptions {
        unitTests {
            // Robolectric renders the screenshots: it needs the merged manifest
            // and the resources of this module (fonts, values).
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
    // UniFFI's records are Kotlin `data class`es with `List` fields: unstable by
    // default, which makes every row of a 4 000-row list recompose on any change.
    stabilityConfigurationFiles.add(rootProject.layout.projectDirectory.file("compose-stability.conf"))
}

dependencies {
    api(platform(libs.androidx.compose.bom))
    api(libs.androidx.compose.ui)
    api(libs.androidx.compose.material3)
    api(libs.androidx.compose.foundation)
    api(libs.androidx.compose.ui.tooling.preview)
    debugImplementation(libs.androidx.compose.ui.tooling)
    api(libs.androidx.annotation)
    // The rows the design system draws are UniFFI records; the types are the
    // bridge's, so a component never re-declares a `TrackRow`.
    api(project(":bridge"))

    // --- JVM tests -----------------------------------------------------------
    // The contrast maths, the motion guard and `ListWindow` are plain Kotlin.
    // The touch-target and screenshot tests need a real Compose measure/layout
    // and real pixels, so they run on Robolectric with its NATIVE graphics
    // backend (Skia on the host): no emulator, no window on the person's
    // display (worker rule 7).
    //
    // Why these coordinates are written here and not in `libs.versions.toml`:
    // that file is created ONCE by MB0 and every lot shares it (worker rule 3).
    // Versions that only this module's tests need therefore live here, in one
    // place, and nowhere else.
    //
    // Why not Paparazzi, which the catalog already pins: MEASURED on this tree
    // (AGP 8.13.2, Kotlin 2.4.21, Compose BOM 2026.05.01) —
    //   * Paparazzi 1.3.5 bundles layoutlib 14 (API 34); `compileSdk = 36`
    //     throws `NoSuchElementException at Renderer.kt:213`, and at 34 the
    //     Compose of this BOM calls framework methods layoutlib 34 does not
    //     have (`StaticLayout$Builder.setUseBoundsForWidth`,
    //     `View.setRequestedFrameRate`), so nothing renders;
    //   * Paparazzi 2.0.0-alpha05.1 fails `:bridge:packageDebugResources`
    //     (`MergedResourceWriterRequest` signature changed under AGP 8.13).
    // Robolectric 4.18 + the Compose test rule runs both the semantics
    // assertions and the screenshots, with no change to the shared files.
    testImplementation(libs.junit)
    // T3: the one golden rule (compare + ATF), and the capture entry point the
    // tests call through it.
    testImplementation(project(":testkit"))
    testImplementation(libs.roborazzi)
    // --- instrumented tests (the device) -------------------------------------
    // One test, `RowSourceTest`: `rememberListWindow`'s adapter over a REAL
    // `ListHandle`, which only exists on a device (JNA + `libnz_ffi.so`).
    androidTestImplementation(libs.junit)
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test:core:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
    testImplementation("org.robolectric:robolectric:4.18")
    testImplementation("androidx.test:core:1.7.0")
    testImplementation("androidx.test.ext:junit:1.3.0")
    testImplementation("androidx.compose.ui:ui-test-junit4")
    // `createComposeRule()` hosts its content in an activity that the test
    // manifest declares; without it Robolectric finds no window to render.
    debugImplementation("androidx.compose.ui:ui-test-manifest")
}

// T3: the goldens live in `mobile/android/screenshots`; `nz.repoRoot` stays
// for the tests that read the SPA's own files.
// and the token test reads `webapp/src/app.css` to prove the Compose table has
// not drifted from the SPA's. Both paths are handed to the test as absolute
// system properties: no test carries `../../..` arithmetic.
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
