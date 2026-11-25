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

// A `@Composable` COMPILED WITHOUT THE COMPOSE COMPILER IS A CRASH, NOT A BUILD ERROR.
//
// The Compose compiler plugin rewrites every `@Composable` function's JVM
// signature (it appends a `Composer` and a `$changed` int), and a caller compiled
// WITH the plugin calls that rewritten signature. A module that declares a
// `@Composable` but does not apply `kotlin.compose` keeps the plain signature:
// kotlinc and R8 accept both sides, and the app dies on the first frame that
// reaches the call with `NoSuchMethodError` — debug and release alike. Measured
// on v1.6.6 (2026-11-25): `:bridge`'s `CoilCovers.Image(String, int)` against
// `:app`'s call to `Image(String, int, Composer, int)`, six crashes in a row on
// the owner's phone as soon as the home screen drew its first cover, while every
// CI job was green (no test drew a real cover through `:bridge`).
//
// The check is per module and runs before `preBuild`, so `assemble*`, `test*`
// and `lint*` all stop on it: a module that annotates a function `@Composable`
// (an annotation at the start of a line, comments excluded) must apply
// `alias(libs.plugins.kotlin.compose)`.
subprojects {
    pluginManager.withPlugin("org.jetbrains.kotlin.android") {
        val composeApplied = provider { pluginManager.hasPlugin("org.jetbrains.kotlin.plugin.compose") }
        val sources = fileTree("src") { include("**/*.kt") }
        val marker = layout.buildDirectory.file("compose-compiler-check/ok")
        val modulePath = path
        val check = tasks.register("checkComposeCompiler") {
            group = "verification"
            description = "Fails when a @Composable is declared in a module without the Compose compiler plugin"
            inputs.files(sources).withPropertyName("sources")
            inputs.property("composeApplied", composeApplied)
            outputs.file(marker)
            doLast {
                if (!composeApplied.get()) {
                    val annotation = Regex("""^\s*@Composable\b""", RegexOption.MULTILINE)
                    val offenders = sources.files.filter { annotation.containsMatchIn(it.readText()) }
                    if (offenders.isNotEmpty()) {
                        throw GradleException(
                            "$modulePath declares @Composable functions without the Compose compiler " +
                                "plugin, so their callers will die with NoSuchMethodError at runtime. " +
                                "Add `alias(libs.plugins.kotlin.compose)` to its plugins block. Files: " +
                                offenders.joinToString { it.relativeTo(rootDir).path },
                        )
                    }
                }
                marker.get().asFile.apply {
                    parentFile.mkdirs()
                    writeText("ok\n")
                }
            }
        }
        tasks.matching { it.name == "preBuild" }.configureEach { dependsOn(check) }
    }
}
