package fr.neezer.mobile.testkit

import com.google.android.apps.common.testing.accessibility.framework.AccessibilityViewCheckResult
import com.google.android.apps.common.testing.accessibility.framework.checks.SpeakableTextPresentCheck
import org.hamcrest.Description
import org.hamcrest.Matcher
import org.hamcrest.TypeSafeMatcher

/**
 * The one ATF finding a screen of this app legitimately produces, named once.
 *
 * `NzSheet`'s tap-anywhere veil (`designsystem/Surfaces.kt`: "the veil takes the
 * tap; it carries no semantics of its own") is a full-window clickable with no
 * speakable text, and the Accessibility Test Framework reports it as
 * `SpeakableTextPresentCheck`. It is a real finding, but the fix belongs to the
 * design system (the veil should not be exposed as an unlabelled target), not to
 * the test lot that renders a sheet — fixing it is a production change T3 does
 * not own.
 *
 * A golden test whose screen opens a sheet passes [sheetVeil]; every other
 * golden runs with no suppression at all. The matcher is exact: the check, the
 * element class the ATF reports for the veil, clickable, and no text and no
 * description.
 */
object AtfSuppressions {
    val sheetVeil: Matcher<in AccessibilityViewCheckResult> =
        object : TypeSafeMatcher<AccessibilityViewCheckResult>() {
            override fun matchesSafely(result: AccessibilityViewCheckResult): Boolean {
                val element = result.element ?: return false
                return result.accessibilityHierarchyCheck == SpeakableTextPresentCheck::class.java &&
                    element.className == "android.view.View" &&
                    element.isClickable &&
                    element.text.isNullOrEmpty() &&
                    element.contentDescription.isNullOrEmpty()
            }

            override fun describeTo(description: Description) {
                description.appendText("NzSheet's unlabelled tap-anywhere veil")
            }
        }
}
