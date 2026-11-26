// :testkit — the ONE JUnit rule a verifying golden wraps (plan T3).
//
// A `:feature:*` module's test depends on this module instead of wiring Roborazzi
// itself, so every golden compares with the same tolerance, writes under the same
// root and runs the same accessibility checks. The module ships test-only types;
// nothing here reaches an APK (`fr.neezer.mobile.testkit` has no production
// class), and `api` is deliberate: the consuming test compiles against the
// Compose test rule and the Roborazzi entry points this module hands it.

plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
    // `ScreenGoldenRule.setContent` takes a `@Composable` lambda: without the
    // Compose compiler its signature and its composable calls are not rewritten,
    // and Kotlin 2.4's invokedynamic lambdas make that a codegen failure (the
    // root script's `checkComposeCompiler` exists for the same class of surprise).
    alias(libs.plugins.kotlin.compose)
    // The plugin reads `-Proborazzi.test.verify=true` per module: applied here too
    // so this module's own configuration (outputDir) is the one the tests see.
    id("io.github.takahirom.roborazzi")
}

android {
    namespace = "fr.neezer.mobile.testkit"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        minSdk = libs.versions.minSdk.get().toInt()
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        buildConfig = false
    }

    testOptions {
        unitTests {
            // A consuming module renders screens with Robolectric: it needs this
            // module's manifest merged so a test has a window to draw into.
            isIncludeAndroidResources = true
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

// The golden root is ONE directory in the repository (`mobile/android/screenshots`,
// plan T3's "single committed golden root"). Both this module's plugin config and
// the rule's `outputDirectoryPath` point there; the rule receives it as an absolute
// system property so no test carries `../../..` arithmetic.
val screenshots = rootProject.layout.projectDirectory.dir("screenshots")

// No `roborazzi { outputDir }` here: the goldens are the repository's ONE root,
// and the consuming module declares it an input. The plugin's own directory stays
// per-module, because its finalize task copies the verify input back — a shared
// directory would race between modules Gradle runs in parallel.
tasks.withType<Test>().configureEach {
    inputs.dir(screenshots)
}

dependencies {
    // The Compose test rule the golden renders through, versionless from the BOM.
    api(platform(libs.androidx.compose.bom))
    api(libs.androidx.compose.ui.test.junit4)
    api(libs.junit)
    // Roborazzi's core, Compose capture, JUnit rule and ATF bridge: `api`, because
    // the rule's signature names them.
    api(libs.roborazzi)
    api(libs.roborazzi.core)
    api(libs.roborazzi.compose)
    api(libs.roborazzi.junit.rule)
    api(libs.roborazzi.accessibility.check)
    api(libs.accessibility.test.framework)
    api(libs.differ)
}

tasks.withType<Test>().configureEach {
    systemProperty("nz.goldens", screenshots.asFile.absolutePath)
    inputs.dir(screenshots)
}
