// :app — the application, the ONE activity, the navigation graph and the manual
// DI graph (docs/agents/mobile.md, "Repository layout").
//
// `fr.neezer.mobile` is the app's OWN id, and that is a deliberate reversal of
// the original MB-SWAP plan (docs/agents/mobile.md, "MB-SWAP — deux apps qui
// coexistent"): the old WebView app (`org.nsupysonic.app`, published as
// `neezer.apk`) keeps its users, and this APK installs BESIDE it instead of
// replacing it. A distinct id is what makes that possible — and the reason the
// MB4 takeover can no longer read the old app's private files: Android gives a
// different package no access to them at all.

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

// WHICH ABIs THIS BUILD PACKS, from `NZ_ABIS` (space-separated, unset = all
// three). CI sets it to `x86_64` on a pull request (a PR run's emulator is
// x86_64, so the other two libraries were built and packed for nobody), and
// `mobile/rust/build-android.sh` reads the same variable — the `.so` set and the
// APK's splits cannot disagree. See docs/agents/mobile.md, « A pull request
// builds ONE ABI » for the measured cost per ABI.
val nzAbis =
    providers
        .environmentVariable("NZ_ABIS")
        .orNull
        ?.split(Regex("\\s+"))
        ?.filter { it.isNotBlank() }
        ?.takeIf { it.isNotEmpty() }
        ?: listOf("arm64-v8a", "armeabi-v7a", "x86_64")

android {
    namespace = "fr.neezer.mobile.app"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        // Its OWN package, beside `org.nsupysonic.app` (the WebView app): both
        // install together, each updates from its own channel, and uninstalling
        // one leaves the other's data alone.
        applicationId = "fr.neezer.mobile"
        minSdk = libs.versions.minSdk.get().toInt()
        targetSdk = libs.versions.targetSdk.get().toInt()

        // No offset any more: the old app's `github.run_number` line belongs to
        // ANOTHER package, and Android only compares codes inside one package.
        // The native APK's versions are the same monotone run numbers, so a
        // `master` build can never be installed over a release.
        versionCode = System.getenv("VERSION_CODE")?.toIntOrNull() ?: 1
        // The tag arrives as `v1.6.3`; the app and `/api/version`'s
        // `android.native.version` spell it `1.6.3`.
        versionName = (System.getenv("VERSION_NAME") ?: "2.0.0").removePrefix("v")

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

    // ONE APK PER ABI, PLUS THE UNIVERSAL ONE (lot « MQ2 — la taille », the note's
    // "Measures"): `libnz_ffi.so` is ~90 % of the APK and a phone runs exactly one
    // of the three ABIs it carried, so the universal APK was paying for two the
    // device can never load. Measured: the universal release APK carried
    // 56 862 808 B of native libraries for a 28 MB split.
    //
    // The app is SIDE-LOADED (`adb install`, `/api/android/apk`), where a Play
    // bundle does not exist: `isUniversalApk = true` keeps a single file that
    // installs on any device, and the per-ABI APKs are what a device downloads
    // when the updater serves its own. Losing x86_64 was the alternative and was
    // refused: the old app shipped it and rule 7 has to keep working there.
    splits {
        abi {
            isEnable = true
            reset()
            include(*nzAbis.toTypedArray())
            isUniversalApk = true
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

    // WHICH BUILD TYPE THE INSTRUMENTED TESTS RUN AGAINST, `debug` by default.
    //
    // `ci-emulator.sh` runs the normal suites with `connectedDebugAndroidTest`
    // (unminified), then asks for `:app:connectedBenchmarkAndroidTest
    // -PnzTestBuildType=benchmark` to run `ReleaseTlsTrustTest` against the
    // RELEASE-SHAPED build (minified by R8, not debuggable, signed with the debug
    // key). A defect that only R8 can cause — v1.6.5's stripped
    // `org.rustls.platformverifier.**` — is invisible in every other shape,
    // which is exactly why it shipped: measured on the owner's phone, the debug
    // app and the smoke were green while the release refused Let's Encrypt.
    testBuildType = providers.gradleProperty("nzTestBuildType").getOrElse("debug")
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

// THE INSTRUMENTED APK'S OWN RULES. Its build type follows the tested one
// (`benchmark` is release-shaped, so the androidTest APK is minified too), and
// `androidx.test` names Error Prone's compile-only annotations in signatures —
// R8 refuses the APK for classes nothing loads. `androidTest-proguard-rules.pro`
// holds that one `-dontwarn`; the release APK is untouched by it.
androidComponents {
    onVariants { variant ->
        variant.androidTest?.proguardFiles?.add(
            layout.projectDirectory.file("androidTest-proguard-rules.pro"),
        )
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
    // The LAN exchange between the person's devices (docs/agents/lan-nodes.md):
    // the « Appareils à proximité » screen and « Sur vos appareils ».
    implementation(project(":feature:lan"))
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

// La garde de l'audit lit les sources (M15) : le chemin lui est donné comme aux
// captures des modules de fonctionnalité, jamais par une arithmétique `../../..`.
val repoRoot = rootProject.layout.projectDirectory.dir("../..")

tasks.withType<Test>().configureEach {
    systemProperty("nz.repoRoot", repoRoot.asFile.absolutePath)
}
