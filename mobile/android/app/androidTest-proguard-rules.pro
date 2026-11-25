# :app's INSTRUMENTED APK keep rules (R8), for the release-shaped build type.
#
# `androidComponents` below adds this file to the androidTest component, which is
# minified whenever the build type it tests is (`benchmark`, the release-shaped
# APK `ci-emulator.sh` runs `ReleaseTlsTrustTest` against). `androidx.test` names
# Error Prone's annotations in method signatures; they are compile-only and never
# on a device, so R8 refuses the APK with "Missing class …" while nothing would
# load them. Warning-level only, and scoped to the test APK: the release APK
# never references them.
-dontwarn com.google.errorprone.annotations.**
