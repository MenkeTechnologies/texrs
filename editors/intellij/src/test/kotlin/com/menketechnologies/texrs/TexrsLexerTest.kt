package com.menketechnologies.texrs

import com.intellij.psi.TokenType
import com.intellij.psi.tree.IElementType
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Tests for [TexrsLexer]. The rules under test are TeX's own — a control word
 * is letters only and swallows the spaces after it, a control symbol is exactly
 * one character however special, `%` runs to the end of the line — and getting
 * any of them wrong mis-colours a document without ever failing loudly.
 */
class TexrsLexerTest {

    private fun tokens(src: String): List<Pair<IElementType?, String>> {
        val lex = TexrsLexer()
        lex.start(src, 0, src.length, 0)
        val out = mutableListOf<Pair<IElementType?, String>>()
        while (lex.tokenType != null) {
            out += lex.tokenType to src.substring(lex.tokenStart, lex.tokenEnd)
            lex.advance()
        }
        return out
    }

    private fun nonWs(src: String) = tokens(src).filter { it.first != TokenType.WHITE_SPACE }

    /** Every token, in order, covers the input exactly once. */
    private fun assertCovers(src: String) {
        val lex = TexrsLexer()
        lex.start(src, 0, src.length, 0)
        var at = 0
        while (lex.tokenType != null) {
            assertEquals("a token starts where the last one ended", at, lex.tokenStart)
            assertTrue("a token makes progress", lex.tokenEnd > lex.tokenStart)
            at = lex.tokenEnd
            lex.advance()
        }
        assertEquals("the tokens cover the whole input", src.length, at)
    }

    @Test
    fun `a per-cent sign comments to the end of the line`() {
        val toks = nonWs("% a comment with \\control and {braces}\n\\relax")
        assertEquals(TexrsTokenTypes.COMMENT, toks[0].first)
        assertTrue(toks[0].second.contains("braces"))
        // The next line is lexed normally: the comment stopped at the newline.
        assertEquals(TexrsTokenTypes.PRIMITIVE, toks[1].first)
    }

    @Test
    fun `a control word is letters only and takes the spaces after it`() {
        val toks = nonWs("\\relax x")
        assertEquals(TexrsTokenTypes.PRIMITIVE, toks[0].first)
        assertEquals("the spaces belong to the control word", "\\relax ", toks[0].second)
        assertEquals(TexrsTokenTypes.TEXT, toks[1].first)
        assertEquals("x", toks[1].second)

        // Digits do not continue a control word: `\count0` is two tokens.
        val counted = nonWs("\\count0=12")
        assertEquals("\\count", counted[0].second)
        assertEquals(TexrsTokenTypes.NUMBER, counted[1].first)
    }

    @Test
    fun `a control symbol is one character however special that character is`() {
        // `\%` is an escaped per-cent, NOT the start of a comment.
        val toks = nonWs("\\% still text\n")
        assertEquals(TexrsTokenTypes.CONTROL_SEQUENCE, toks[0].first)
        assertEquals("\\%", toks[0].second)
        assertEquals(TexrsTokenTypes.TEXT, toks[1].first)

        for (src in listOf("\\\\", "\\{", "\\}", "\\$", "\\#", "\\&")) {
            val one = nonWs(src)
            assertEquals("$src is one token", 1, one.size)
            assertEquals(src, one[0].second)
        }
    }

    @Test
    fun `the primitives texrs implements are told apart from macros a document defines`() {
        val toks = nonWs("\\def\\greet#1{\\message{#1}}")
        assertEquals(TexrsTokenTypes.PRIMITIVE, toks[0].first)
        assertEquals(
            "a name texrs does not carry is a control sequence like any other",
            TexrsTokenTypes.CONTROL_SEQUENCE,
            toks[1].first,
        )
        assertEquals("\\greet", toks[1].second)
        assertEquals(TexrsTokenTypes.PARAMETER, toks[2].first)
        assertEquals("#1", toks[2].second)
        assertEquals(TexrsTokenTypes.BEGIN_GROUP, toks[3].first)
        assertEquals(TexrsTokenTypes.PRIMITIVE, toks[4].first)
    }

    @Test
    fun `conditionals and group markers are block keywords`() {
        val toks = nonWs("\\ifnum1<2 \\else \\fi \\begingroup \\endgroup")
        val kinds = toks.map { it.first }
        assertEquals(TexrsTokenTypes.BLOCK_KEYWORD, kinds[0])
        assertTrue(
            "every block marker is one",
            toks.filter { it.second.trimEnd() in setOf("\\else", "\\fi", "\\begingroup", "\\endgroup") }
                .all { it.first == TexrsTokenTypes.BLOCK_KEYWORD },
        )
    }

    @Test
    fun `the category-code characters each get their own token`() {
        val toks = nonWs("{}\$&#^_~")
        assertEquals(
            listOf(
                TexrsTokenTypes.BEGIN_GROUP,
                TexrsTokenTypes.END_GROUP,
                TexrsTokenTypes.MATH_SHIFT,
                TexrsTokenTypes.ALIGN_TAB,
                TexrsTokenTypes.PARAMETER,
                TexrsTokenTypes.SUPERSCRIPT,
                TexrsTokenTypes.SUBSCRIPT,
                TexrsTokenTypes.ACTIVE_CHAR,
            ),
            toks.map { it.first },
        )
    }

    @Test
    fun `caret caret notation is one character to the engine and one token here`() {
        val toks = nonWs("^^M x")
        assertEquals(TexrsTokenTypes.OTHER, toks[0].first)
        assertEquals("^^M", toks[0].second)
        // A lone caret is still the superscript character.
        assertEquals(TexrsTokenTypes.SUPERSCRIPT, nonWs("^2")[0].first)
    }

    @Test
    fun `the lexer covers its input whatever it is given`() {
        assertCovers("")
        assertCovers("\\")
        assertCovers("%")
        assertCovers("^")
        assertCovers("^^")
        assertCovers("\\catcode`\\{=1 \\catcode`\\}=2\n\\message{hi}\n\\end\n")
        assertCovers("text with \$math\$ and a % comment\nand é accents — dashes\n")
    }
}
