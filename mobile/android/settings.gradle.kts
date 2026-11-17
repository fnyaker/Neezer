pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    // Every dependency is declared where it is USED, never in a module's own
    // `repositories {}`: one resolution mode, one lockfile story (the desktop's
    // rule, same reason).
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
        // `rustls-platform-verifier`'s Android side is an AAR published to the
        // project's own Maven archive (its README's recipe). `:bridge` resolves
        // its version from `mobile/rust/Cargo.lock`, so the Kotlin component and
        // the Rust crate can never disagree (MB2).
        maven {
            url = uri(
                "https://github.com/rustls/rustls-platform-verifier/raw/" +
                    "maven-archive/android-release-support/maven/"
            )
            content { includeGroup("org.rustls") }
        }
    }
}

rootProject.name = "neezer-mobile"

// The app, and the modules every feature builds on.
include(":app")
include(":bridge")
include(":designsystem")
include(":playback")
include(":car")
include(":baselineprofile")

// One Gradle module per feature lot (docs/agents/mobile.md, "Module and file
// ownership"): disjoint files, so two lots never edit the same module.
include(":feature:setup")
include(":feature:onboarding")
include(":feature:home")
include(":feature:search")
include(":feature:entity")
include(":feature:library")
include(":feature:playlist")
include(":feature:trackactions")
include(":feature:nowplaying")
include(":feature:audiofx")
include(":feature:podcasts")
include(":feature:discovery")
include(":feature:offline")
include(":feature:settings")
include(":feature:admin")
include(":feature:genres")
include(":feature:remote")
include(":feature:party")
include(":feature:viz")
include(":feature:worlds")
