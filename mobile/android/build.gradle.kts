// The root project declares the plugins ONCE, `apply false`: a module applies
// the ones it needs by alias (`alias(libs.plugins.android.library)`), so a
// version is never written twice (gradle/libs.versions.toml is the only place).
//
// MB0 created this file. A lot that needs another plugin declares it here and
// asks the orchestrator first: this file is shared by every module.

plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.android.library) apply false
    alias(libs.plugins.android.test) apply false
    alias(libs.plugins.kotlin.android) apply false
    alias(libs.plugins.kotlin.compose) apply false
    alias(libs.plugins.kotlin.serialization) apply false
    alias(libs.plugins.baselineprofile) apply false
    alias(libs.plugins.paparazzi) apply false
}
