// :app — the application, the ONE activity, the navigation graph and the manual
// DI graph (docs/agents/mobile.md, "Repository layout").
//
// applicationId stays `org.nsupysonic.app` and the signing key stays the old
// app's: the new APK installs as an UPDATE over `neezer.apk`, keeps its data
// directory (the migration reads from it) and keeps its home-screen icon and
// pinned shortcuts working (rule 7).

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

android {
    namespace = "fr.neezer.mobile.app"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        applicationId = "org.nsupysonic.app"
        minSdk = libs.versions.minSdk.get().toInt()
        targetSdk = libs.versions.targetSdk.get().toInt()

        // The old app shipped `versionCode = github.run_number` (android.yaml).
        // Android refuses an update whose code is not HIGHER, so the new line
        // starts at 2 000 000 and adds the same run number: no release of the
        // WebView app can ever be above it by accident (docs/agents/mobile.md,
        // "Pitfalls known before the first line").
        versionCode = 2_000_000 + (System.getenv("VERSION_CODE")?.toIntOrNull() ?: 1)
        versionName = System.getenv("VERSION_NAME") ?: "2.0.0"

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    // Exactly as android/app/build.gradle.kts: v1, v2 AND v3, on the debug
    // fallback too. AGP decides v1 from the minSdk when left to itself, and a
    // minSdk-26 build then ships v2-only — measured on the old app's v1.3.1,
    // and `apksigner verify` hides it unless asked `--min-sdk-version 23`.
    signingConfigs {
        create("release") {
            val storePath = System.getenv("ANDROID_KEYSTORE_FILE")
            if (storePath != null) {
                storeFile = file(storePath)
                storePassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("ANDROID_KEY_ALIAS")
                keyPassword = System.getenv("ANDROID_KEY_PASSWORD")
            }
            enableV1Signing = true
            enableV2Signing = true
            enableV3Signing = true
        }
        getByName("debug") {
            enableV1Signing = true
            enableV2Signing = true
            enableV3Signing = true
        }
    }

    buildTypes {
        release {
            // R8 full mode and the resources shrunk: the APK is what the person
            // downloads over a mobile link (rule 3, "size and cold start").
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig =
                if (System.getenv("ANDROID_KEYSTORE_FILE") != null) {
                    signingConfigs.getByName("release")
                } else {
                    signingConfigs.getByName("debug")
                }
        }
        debug {
            versionNameSuffix = "-debug"
        }
        // :baselineprofile's target (MQ1/MQ2): a release-shaped build that can be
        // profiled, signed so it installs beside nothing.
        create("benchmark") {
            initWith(getByName("release"))
            signingConfig = signingConfigs.getByName("debug")
            matchingFallbacks += listOf("release")
            isDebuggable = false
            // The BenchmarkRule refuses a debuggable or profileable=false app.
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    lint {
        // Fully green CI means lint's own findings are read, not silenced: an
        // error-level check fails the job, a warning does not.
        abortOnError = true
        warningsAsErrors = false
        checkDependencies = true
    }

    testOptions {
        unitTests.isIncludeAndroidResources = false
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
    implementation(project(":playback"))
    implementation(project(":car"))

    // Every feature module: `:app` owns the navigation graph and the shell, and
    // it is the only module that sees all of them (docs/agents/mobile.md, MB0's
    // module dependency list).
    implementation(project(":feature:setup"))
    implementation(project(":feature:onboarding"))
    implementation(project(":feature:home"))
    implementation(project(":feature:search"))
    implementation(project(":feature:entity"))
    implementation(project(":feature:library"))
    implementation(project(":feature:playlist"))
    implementation(project(":feature:trackactions"))
    implementation(project(":feature:nowplaying"))
    implementation(project(":feature:audiofx"))
    implementation(project(":feature:podcasts"))
    implementation(project(":feature:discovery"))
    implementation(project(":feature:offline"))
    implementation(project(":feature:settings"))
    implementation(project(":feature:admin"))
    implementation(project(":feature:genres"))
    implementation(project(":feature:remote"))
    implementation(project(":feature:party"))
    implementation(project(":feature:viz"))
    implementation(project(":feature:worlds"))

    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.lifecycle.process)
    implementation(libs.androidx.navigation.compose)
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.kotlinx.coroutines.android)
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.foundation)
    implementation(libs.androidx.compose.ui.tooling.preview)
    debugImplementation(libs.androidx.compose.ui.tooling)
    // The Baseline Profile is installed on first run, so the start is the same
    // on a cold device as on a warmed one (MQ1 generates it).
    implementation(libs.androidx.profileinstaller)

    testImplementation(libs.junit)

    // Instrumented tests (MQ1): the shell's smoke on a real device, and the
    // rules every screen lot's Compose test uses. A module's own androidTest
    // declares what it needs as well; :app is the one that proves the chain
    // (Gradle → APK → instrumentation → Compose semantics) for all of them.
    androidTestImplementation(libs.androidx.test.core)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.rules)
    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(libs.androidx.test.uiautomator)
    androidTestImplementation(platform(libs.androidx.compose.bom))
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)
}
