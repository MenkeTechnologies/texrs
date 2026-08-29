package com.menketechnologies.texrs

import com.intellij.lexer.LexerBase
import com.intellij.psi.TokenType
import com.intellij.psi.tree.IElementType

/**
 * A hand-rolled TeX lexer, classifying by category code exactly as the engine
 * does (`tex.web` §207; `src/catcode.rs` in this repo).
 *
 * It assumes plain TeX's table rather than INITEX's, because that is what a
 * `.tex` file open in an editor almost always is — INITEX leaves `{`, `}`, `$`,
 * `&`, `#`, `^`, `_` and `~` ordinary, which would colour a normal document as
 * one long run of text. A document that reassigns a catcode is coloured by the
 * table it was opened with; it still lexes into whole tokens, because no
 * classification here can make the scanner lose its place.
 *
 * Two rules are worth stating because getting them wrong is invisible until it
 * is not:
 *
 *  * A control word is a backslash followed by *letters only*, and it swallows
 *    the spaces after it (`tex.web` §354). A control *symbol* is a backslash
 *    and exactly one non-letter, so `\%` is one token and not a comment.
 *  * `^^X` is a single character to the engine (§352), so it is one token here.
 */
class TexrsLexer : LexerBase() {
    private var buffer: CharSequence = ""
    private var bufferEnd = 0
    private var tokenStart = 0
    private var tokenEnd = 0
    private var currentToken: IElementType? = null

    override fun start(buffer: CharSequence, startOffset: Int, endOffset: Int, initialState: Int) {
        this.buffer = buffer
        this.bufferEnd = endOffset
        this.tokenStart = startOffset
        this.tokenEnd = startOffset
        advance()
    }

    override fun getState(): Int = 0
    override fun getTokenType(): IElementType? = currentToken
    override fun getTokenStart(): Int = tokenStart
    override fun getTokenEnd(): Int = tokenEnd
    override fun getBufferSequence(): CharSequence = buffer
    override fun getBufferEnd(): Int = bufferEnd

    override fun advance() {
        tokenStart = tokenEnd
        if (tokenStart >= bufferEnd) {
            currentToken = null
            return
        }
        val c = buffer[tokenStart]
        when {
            c == '%' -> comment()
            c == '\\' -> controlSequence()
            c == '{' -> single(TexrsTokenTypes.BEGIN_GROUP)
            c == '}' -> single(TexrsTokenTypes.END_GROUP)
            c == '$' -> single(TexrsTokenTypes.MATH_SHIFT)
            c == '&' -> single(TexrsTokenTypes.ALIGN_TAB)
            c == '#' -> parameter()
            c == '^' -> superscriptOrDoubled()
            c == '_' -> single(TexrsTokenTypes.SUBSCRIPT)
            c == '~' -> single(TexrsTokenTypes.ACTIVE_CHAR)
            c.isWhitespace() -> whitespace()
            c.isDigit() -> digits()
            isLetter(c) -> letters()
            else -> single(TexrsTokenTypes.OTHER)
        }
    }

    /// `%` runs to the end of the line, and the line ending goes with it —
    /// TeX discards it too, which is why `a%\nb` reads as `ab`.
    private fun comment() {
        var at = tokenStart + 1
        while (at < bufferEnd && buffer[at] != '\n') at++
        emit(at, TexrsTokenTypes.COMMENT)
    }

    /**
     * A control word (`\relax`) or a control symbol (`\%`, `\\`).
     *
     * The spaces after a control word are part of it: TeX skips them, so
     * `\par  x` is one control sequence and then `x`. A backslash at the very
     * end of the file is a token on its own rather than a scanner that runs off
     * the end.
     */
    private fun controlSequence() {
        var at = tokenStart + 1
        if (at >= bufferEnd) {
            emit(at, TexrsTokenTypes.CONTROL_SEQUENCE)
            return
        }
        val first = buffer[at]
        if (!isLetter(first)) {
            // A control symbol is the backslash and exactly one character,
            // however special that character would be on its own.
            emit(at + 1, TexrsTokenTypes.CONTROL_SEQUENCE)
            return
        }
        while (at < bufferEnd && isLetter(buffer[at])) at++
        val name = buffer.subSequence(tokenStart + 1, at).toString()
        // The spaces a control word swallows belong to the token; a newline
        // does not, so the following line still starts in state N.
        var after = at
        while (after < bufferEnd && (buffer[after] == ' ' || buffer[after] == '\t')) after++
        val type = when {
            name in BLOCK_KEYWORDS -> TexrsTokenTypes.BLOCK_KEYWORD
            name in PRIMITIVES -> TexrsTokenTypes.PRIMITIVE
            else -> TexrsTokenTypes.CONTROL_SEQUENCE
        }
        emit(after, type)
    }

    /// `#1` … `#9` is a parameter reference; a bare `#` is the parameter
    /// character itself, which is what a `\def` argument list is made of.
    private fun parameter() {
        val at = tokenStart + 1
        val end = if (at < bufferEnd && buffer[at].isDigit()) at + 1 else at
        emit(end, TexrsTokenTypes.PARAMETER)
    }

    /// `^^X` is one character to the engine, so it is one token here; a lone
    /// `^` is the superscript character.
    private fun superscriptOrDoubled() {
        val second = tokenStart + 1
        if (second < bufferEnd && buffer[second] == '^' && second + 1 < bufferEnd) {
            emit(second + 2, TexrsTokenTypes.OTHER)
            return
        }
        single(TexrsTokenTypes.SUPERSCRIPT)
    }

    private fun whitespace() {
        var at = tokenStart
        while (at < bufferEnd && buffer[at].isWhitespace()) at++
        emit(at, TokenType.WHITE_SPACE)
    }

    private fun digits() {
        var at = tokenStart
        while (at < bufferEnd && buffer[at].isDigit()) at++
        emit(at, TexrsTokenTypes.NUMBER)
    }

    private fun letters() {
        var at = tokenStart
        while (at < bufferEnd && isLetter(buffer[at])) at++
        emit(at, TexrsTokenTypes.TEXT)
    }

    private fun single(type: IElementType) = emit(tokenStart + 1, type)

    private fun emit(end: Int, type: IElementType) {
        tokenEnd = end.coerceAtMost(bufferEnd)
        currentToken = type
    }

    /// Catcode 11. TeX's own letters are ASCII only — `\é` is a control symbol,
    /// not a control word — and following that keeps `\ém` lexing the way the
    /// engine reads it.
    private fun isLetter(c: Char): Boolean = c in 'a'..'z' || c in 'A'..'Z'

    companion object {
        /// The control sequences texrs's expander implements, from `expand.rs`
        /// and `lower.rs`. A name here is coloured as a primitive; anything else
        /// is a control sequence like any other, since a document may define it.
        @JvmField
        val PRIMITIVES: Set<String> = setOf(
            "advance", "catcode", "count", "csname", "def", "divide", "edef",
            "end", "endcsname", "expandafter", "gdef", "global", "ignorespaces",
            "let", "message", "multiply", "noexpand", "number", "par", "relax",
            "string", "the", "xdef",
        )

        /// The conditionals and group markers, which open or close something
        /// and so drive Complete Current Statement.
        @JvmField
        val BLOCK_KEYWORDS: Set<String> = setOf(
            "begingroup", "endgroup", "if", "ifcase", "ifcat", "ifcsname",
            "ifdefined", "ifdim", "ifeof", "iffalse", "ifhbox", "ifhmode",
            "ifinner", "ifmmode", "ifnum", "ifodd", "iftrue", "ifvbox",
            "ifvmode", "ifvoid", "ifx", "else", "or", "fi",
        )
    }
}
