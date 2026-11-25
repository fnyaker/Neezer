// :car — the car overlay, the quick-settings tile and the triggers, ported from
// `android/` (docs/agents/mobile.md, lot MB24).
//
// The components live in `fr.neezer.mobile.car`, like the rest of this app: the
// old `org.nsupysonic.app.*` names existed only because the first plan installed
// this APK OVER the WebView app and had to keep its tile and pinned shortcuts
// alive. With its own `applicationId` there is nothing to preserve, and a native
// app that never spells the old package is one a person (and a reviewer) can tell
// apart at a glance. MB24's two seams are unchanged: a Media3 `MediaController`
// instead of `PlayerService.transport/state`, and `NzCore` calls instead of
// `__nsCarPlay`/`__nsCarFav`.

plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
}

android {
    // The module's namespace, the classes and the manifest's component names
    // are all this app's: no `org.nsupysonic.app` survives in the native APK.
    namespace = "fr.neezer.mobile.car"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        minSdk = libs.versions.minSdk.get().toInt()
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
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

    testOptions {
        unitTests {
            isIncludeAndroidResources = true
        }
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
    // The old app's car settings screen (a View host, not Compose): `ComponentActivity`
    // for `registerForActivityResult`, on the APP's own platform window theme. Never
    // `AppCompatActivity`: that one refuses any theme that is not a `Theme.AppCompat`
    // descendant, and this app deliberately ships none (see `:app`'s themes.xml) —
    // it crashed the screen on the very first `setContentView`.
    implementation("androidx.activity:activity:1.11.0")
    // A resolution FLOOR, not a use: `play-services-basement` (through
    // play-services-location) drags `androidx.fragment:fragment:1.1.0`, and
    // `androidx.activity`'s own lint refuses `registerForActivityResult` below
    // 1.3.0 (`InvalidFragmentVersionForActivityResult`: FragmentActivity 1.2
    // did not call `super.onRequestPermissionsResult()`). appcompat 1.7.0 used
    // to raise that floor transitively; :car no longer needs appcompat, so the
    // floor is stated here.
    constraints {
        implementation("androidx.fragment:fragment:1.8.9")
    }
    // The speed lock and the GPS-app trigger: `FusedLocationProviderClient` is
    // the normal path and `CarMotion` falls back to `LocationManager` on a
    // device without Play services, exactly as `android/`'s manifest comment
    // said ("nothing else here may depend on GMS").
    implementation("com.google.android.gms:play-services-location:21.3.0")

    // The ported policies: pure Kotlin, no device. Robolectric is for the ONE
    // screen this module owns: `CarSettingsActivityLaunchTest` inflates it under
    // the app's real window theme, the check that catches an `AppCompatActivity`
    // (or a theme) that refuses to paint at all.
    testImplementation(libs.junit)
    testImplementation("org.robolectric:robolectric:4.18")
    // The device test that opens the settings and manoeuvres every control.
    androidTestImplementation("androidx.test:core:1.7.0")
    androidTestImplementation("androidx.test:runner:1.7.0")
    androidTestImplementation("androidx.test.ext:junit:1.3.0")
}
