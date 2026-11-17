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
        // `rustls-platform-verifier`'s Android side is an AAR that ships inside
        // the crate's own local Maven repository. `:bridge` adds that directory
        // when it exists (MB2 wires it from `cargo metadata`); the line stays
        // commented until then so a missing path is not a configuration error.
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
