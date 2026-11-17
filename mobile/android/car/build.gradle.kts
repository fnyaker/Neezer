// :car — the car overlay, the quick-settings tile and the triggers, ported from
// `android/` (docs/agents/mobile.md, lot MB24).
//
// MB0 creates the module, its manifest and the component classes under their OLD
// fully-qualified names (`org.nsupysonic.app.CarModeService`, `CarTileService`,
// `CarSettingsActivity`, the receivers, the job service): a quick-settings tile a
// person already added points at those names, and an update must not orphan it.
// MB24 fills them; the two seams it changes are named in the note (a Media3
// `MediaController` instead of `PlayerService.transport/state`, and `NzCore`
// calls instead of `__nsCarPlay`/`__nsCarFav`).

plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
}

android {
    namespace = "fr.neezer.mobile.car"
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
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

dependencies {
    implementation(project(":bridge"))
    implementation(libs.androidx.media3.session)
    implementation(libs.kotlinx.coroutines.android)
}
