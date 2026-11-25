package fr.neezer.mobile.bridge

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

/**
 * Le dernier incident s'écrit et se relit, dans le vrai `filesDir` de l'app.
 *
 * Robolectric plutôt qu'un simple JUnit : `CrashRecord` ne fait rien d'autre que
 * `File(context.filesDir, …)`, et `android.jar` rend des méthodes qui lèvent
 * « Stub! » — le test aurait vérifié une maquette, pas le chemin qui écrit.
 *
 * Ce que la carte d'Assistance lit est exactement `read()` : le format asserté
 * ici (`thread=` présent, pile complète, `clear` = plus rien) est celui qu'un
 * plantage réel produit, parce que `CrashLog` appelle le même `write`.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class CrashRecordTest {
    private val context = RuntimeEnvironment.getApplication()

    @Test
    fun writeThenReadKeepsTheThreadAndTheStack() {
        CrashRecord.clear(context)
        CrashRecord.write(
            context = context,
            thread = "main",
            stack = "java.lang.IllegalStateException: banc\n\tat fr.neezer.mobile.Foo.bar(Foo.kt:12)",
        )

        val trace = CrashRecord.read(context)
        assertNotNull("la trace doit être écrite", trace)
        val text = trace!!

        assertTrue("la trace commence par son horodatage", text.startsWith("time="))
        assertTrue("le fil est nommé", text.contains("thread=main"))
        assertTrue(
            "la pile entière est là, lignes d'appel comprises",
            text.contains("java.lang.IllegalStateException: banc") &&
                text.contains("at fr.neezer.mobile.Foo.bar(Foo.kt:12)"),
        )
    }

    @Test
    fun clearThenReadIsEmpty() {
        CrashRecord.write(context, "RenderThread", "java.lang.Error: banc")

        CrashRecord.clear(context)

        assertNull("après « Effacer », la carte n'a plus rien à lire", CrashRecord.read(context))
    }

    /** Une seule entrée : le second plantage remplace le premier. */
    @Test
    fun theLatestCrashReplacesThePreviousOne() {
        CrashRecord.write(context, "main", "java.lang.Error: premier")
        CrashRecord.write(context, "worker-3", "java.lang.Error: second")

        val trace = CrashRecord.read(context)
        assertNotNull("la trace doit être écrite", trace)
        val text = trace!!

        assertTrue("c'est le SECOND incident qui reste", text.contains("thread=worker-3"))
        assertEquals("le fichier ne garde qu'un incident", 1, text.split("time=").size - 1)
    }
}
