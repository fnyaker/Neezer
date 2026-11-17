# JNA and UniFFI keep rules (docs/agents/mobile.md, MB0 "Contract": "R8 full
# mode, keep rules for `com.sun.jna.**` and `uniffi.**`").
#
# The bindings reach the library through JNA reflection and through callbacks
# that KOTLIN implements and RUST calls back into: R8 cannot see either edge, so
# it must not rename or remove them. A consumer rule file (rather than :app's own)
# keeps this true for any app that embeds the module.

-keep class com.sun.jna.** { *; }
-keep interface com.sun.jna.** { *; }
-keep class uniffi.** { *; }
-keepclassmembers class uniffi.** { *; }
-keep class fr.neezer.mobile.bridge.** { *; }

# JNA reaches optional platform classes it can live without: warning-level only,
# an error-level `-dontwarn` would hide a real missing class.
-dontwarn java.awt.**
-dontwarn javax.swing.**
-dontwarn sun.misc.**
-dontwarn com.sun.jna.**
