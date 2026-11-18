// :car — the car overlay, the quick-settings tile and the triggers, ported from
// `android/` (docs/agents/mobile.md, lot MB24).
//
// MB0 created the module, its manifest and the component classes under their OLD
// fully-qualified names (`org.nsupysonic.app.CarModeService`, `CarTileService`,
// `CarSettingsActivity`, the receivers, the job service): a quick-settings tile a
// person already added points at those names, and an update must not orphan it.
// MB24 filled them; the two seams it changed are named in the note (a Media3
// `MediaController` instead of `PlayerService.transport/state`, and `NzCore`
// calls instead of `__nsCarPlay`/`__nsCarFav`).

plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
}

android {
    // The MODULE's namespace stays MB0's; the CLASSES keep the old app's package
    // (`org.nsupysonic.app`), which is what the manifest's component names, the
    // person's quick-settings tile and the `car` preferences file name.
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

    // The three dependencies below are literal on purpose: the version catalog
    // belongs to MB0 (worker rule 3), and a coordinate this module alone uses is
    // not worth a shared edit (`:feature/settings` set the same precedent for
    // its test pins).
    lint {
        disable += "UseTomlInstead"
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

dependencies {
    implementation(project(":bridge"))
    // `NzMediaService` and `CoverArt`: the car binds the app's ONE media session
    // through a `MediaController` (MB24's seam) and reads the cover URL out of
    // its `MediaMetadata`, so the two cannot drift apart.
    implementation(project(":playback"))
    // The Sillon faces (Bricolage Grotesque, Instrument Sans, JetBrains Mono):
    // bundled once by MB1, drawn here by the car's own `Typeface`s.
    implementation(project(":designsystem"))
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.media3.session)
    implementation(libs.kotlinx.coroutines.android)
    // The old app's car settings screen (a View host, not Compose): appcompat for
    // `AppCompatActivity` + `registerForActivityResult`.
    implementation("androidx.appcompat:appcompat:1.7.0")
    // The speed lock and the GPS-app trigger: `FusedLocationProviderClient` is
    // the normal path and `CarMotion` falls back to `LocationManager` on a
    // device without Play services, exactly as `android/`'s manifest comment
    // said ("nothing else here may depend on GMS").
    implementation("com.google.android.gms:play-services-location:21.3.0")

    // The ported policies: pure Kotlin, no device, no Robolectric.
    testImplementation(libs.junit)
}
