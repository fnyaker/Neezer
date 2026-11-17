# :app's own keep rules (R8 full mode).
#
# :bridge's `consumer-rules.pro` already keeps JNA and `uniffi.**` for every
# module that embeds it. What is left here is what R8 cannot see: the components
# the MANIFEST names (it keeps manifest-referenced classes, but not the classes
# they build by name), and the callback interfaces Rust calls back into.
#
# The car mode's components are declared under their OLD fully-qualified names
# in :car's manifest, so the app's package prefix must survive too (a quick
# settings tile the person added points at it).

-keep class org.nsupysonic.app.** { *; }
-keep class fr.neezer.mobile.app.** { *; }

# UniFFI callbacks are Kotlin objects RUST invokes: R8 must not rename or strip
# their bridge methods.
-keep class * implements uniffi.nz_ffi.SessionListener { *; }
-keep class * implements uniffi.nz_ffi.PlayerListener { *; }

# The media session is reached by Media3 through the framework, by name.
-keep class fr.neezer.mobile.playback.** { *; }

# Stop R8 from warning about the optional platform bits JNA reaches for.
-dontwarn org.bouncycastle.**
-dontwarn org.conscrypt.**
-dontwarn org.openjsse.**
