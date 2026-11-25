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

# THE PLATFORM TLS VERIFIER (measured on v1.6.5, the release APK only).
#
# `rustls-platform-verifier`'s Android half is a Rust crate that calls the JVM
# class `org.rustls.platformverifier.CertificateVerifier` through JNI, by NAME:
# R8 cannot see that edge, so a minified build strips or renames the class and
# every system-trust handshake then fails — while debug builds and the
# unminified emulator smoke stay green, which is exactly how v1.6.5 shipped a
# release whose "Vérifier le certificat" said the server was offline and whose
# login refused a plain Let's Encrypt certificate.
#
# The rule is the upstream README's own (`org.rustls:rustls-platform-verifier`,
# section "Proguard"): `includedescriptorclasses` keeps the signatures the JNI
# bridge reflects on. `mobile.yaml` fails the `apk` job when the R8 mapping no
# longer shows these classes kept, so a rule file that drifts is a red check and
# not another silent release.
-keep, includedescriptorclasses class org.rustls.platformverifier.** { *; }

# JNA reaches optional platform classes it can live without: warning-level only,
# an error-level `-dontwarn` would hide a real missing class.
-dontwarn java.awt.**
-dontwarn javax.swing.**
-dontwarn sun.misc.**
-dontwarn com.sun.jna.**
