package org.nsupysonic.app

import android.content.SharedPreferences
import java.lang.reflect.Proxy

/**
 * A [Prefs] backed by an in-memory map, for the JVM tests that exercise
 * [Net] and the origin rule without an Android runtime. Only the getters
 * those paths read are implemented; a setter/edit call is a test bug and
 * would return null from the proxy.
 */
internal fun testPrefs(vararg values: Pair<String, Any?>): Prefs {
    val map = values.toMap()
    val sp = Proxy.newProxyInstance(
        SharedPreferences::class.java.classLoader,
        arrayOf(SharedPreferences::class.java),
    ) { _, method, args ->
        val key = args?.getOrNull(0) as? String
        when (method.name) {
            "getString" -> map[key] as? String ?: args?.getOrNull(1)
            "getBoolean" -> map[key] as? Boolean ?: (args?.getOrNull(1) as? Boolean ?: false)
            "getInt" -> map[key] as? Int ?: (args?.getOrNull(1) ?: 0)
            "getLong" -> map[key] as? Long ?: (args?.getOrNull(1) ?: 0L)
            "getFloat" -> map[key] as? Float ?: (args?.getOrNull(1) ?: 0f)
            "getStringSet" -> (map[key] as? Collection<*>)?.filterIsInstance<String>()?.toSet()
                ?: emptySet<String>()
            "contains" -> map.containsKey(key)
            "getAll" -> map
            else -> null
        }
    } as SharedPreferences
    return Prefs(sp)
}
