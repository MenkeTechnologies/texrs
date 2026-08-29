package com.menketechnologies.texrs

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The pure half of [TexrsSettings] — extension parsing and the executable
 * fallback — on a freshly constructed instance, so no ApplicationManager is
 * needed. `getInstance()` is the platform's job and is not exercised here.
 */
class TexrsSettingsTest {

    private fun fresh(extensions: String): TexrsSettings =
        TexrsSettings().apply { fileExtensions = extensions }

    @Test
    fun `the default set is tex`() {
        assertTrue("tex" in TexrsSettings().supportedExtensions())
    }

    @Test
    fun `extensions are parsed from commas spaces or semicolons and lose their dots`() {
        val s = fresh("tex, .sty;cls ltx")
        assertEquals(setOf("tex", "sty", "cls", "ltx"), s.supportedExtensions().toSet())
    }

    @Test
    fun `a file is claimed by its extension and nothing else`() {
        val s = fresh("tex, sty")
        assertTrue(s.isSupportedFile("doc.tex", "tex"))
        assertTrue(s.isSupportedFile("macros.sty", "sty"))
        assertFalse(s.isSupportedFile("notes.txt", "txt"))
        // A file the IDE gives no extension for is still matched on its name.
        assertTrue(s.isSupportedFile("doc.tex", null))
        assertFalse(s.isSupportedFile("Makefile", null))
    }

    @Test
    fun `a blank executable falls back to the one on PATH`() {
        val s = TexrsSettings()
        assertEquals("texrs", s.executable())
        s.texrsExecutable = "   "
        assertEquals("texrs", s.executable())
        s.texrsExecutable = "/opt/texrs/bin/texrs"
        assertEquals("/opt/texrs/bin/texrs", s.executable())
    }
}
