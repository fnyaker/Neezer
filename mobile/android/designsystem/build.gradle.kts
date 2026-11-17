// :designsystem — Sillon in Compose (docs/agents/mobile.md, MB1).
//
// MB0 creates the module and a MINIMAL theme so the shell and the 20 feature
// modules compile against it; MB1 owns everything under `designsystem/**` from
// here (exact tokens of webapp/src/app.css, the fonts, the motion tokens with
// the eco/reduced guard, and every component).

plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "fr.neezer.mobile.designsystem"
    compileSdk = libs.versions.compileSdk.get().toInt()

    defaultConfig {
        minSdk = libs.versions.minSdk.get().toInt()
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
        buildConfig = false
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
}
