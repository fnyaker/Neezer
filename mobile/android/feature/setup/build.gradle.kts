// :feature/setup — the Serveur et connexion module (docs/agents/mobile.md, lot MB4).
//
// One Gradle module per feature lot, disjoint files: a lot fills ITS module and
// edits no other. MB0 created the module, its stub entry composable and the
// signatures the shell and the other feature modules already call.

plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "fr.neezer.mobile.feature.setup"
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

    // `org.json` is a JVM test coordinate only (see the dependency block): the
    // catalog is MB0's and shared by every lot, so the one coordinate this
    // module's tests need is written below and not in the catalog (worker rule 3).
    lint {
        disable += "UseTomlInstead"
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
    // `rememberLauncherForActivityResult`: DeezerLoginActivity answers the very
    // screen that opened it (the Activity Result contract), which is what keeps
    // an ARL from travelling through the whole app.
    implementation(libs.androidx.activity.compose)
    // The FileProvider the manifest names is androidx.core's (androidx.core.content.FileProvider).
    implementation(libs.androidx.core.ktx)
    // `org.json` on the JVM tests too: the migration's page payload and the old
    // app's sidecars are read with the platform's own parser.
    testImplementation(libs.junit)
    // `org.json` is Android's own class and the unit-test stub returns nothing:
    // the migration's page payload and the old sidecars are read with it, so the
    // JVM tests get the real parser (the same API, the same behaviour).
    testImplementation("org.json:json:20260814")
}
