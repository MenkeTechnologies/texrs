package com.menketechnologies.texrs

import com.intellij.psi.tree.IElementType

class TexrsTokenType(debugName: String) : IElementType(debugName, TexrsLanguage)

/**
 * Token categories for TeX, named after the category codes the engine works in
 * (`tex.web` §207, and `catcode.rs` in this repo). Highlighting a TeX file is
 * catcode classification with the primitives picked out of it, so the token set
 * follows the sixteen classes rather than inventing a parallel vocabulary.
 *
 * A file's real catcodes are mutable — `\catcode`\@=11` makes `@` a letter —
 * and no editor can know them without running the document. What this lexer
 * assumes is plain TeX's table, which is what a `.tex` file in an editor almost
 * always is; a document that reassigns a catcode is coloured by the old meaning
 * and still lexes into whole tokens rather than falling apart.
 *
 * When adding a token here, add the matching case in
 * [TexrsSyntaxHighlighter.getTokenHighlights] and the entry in
 * [TexrsColorSettingsPage].
 */
object TexrsTokenTypes {
    /// `%` to end of line (catcode 14).
    @JvmField val COMMENT = TexrsTokenType("TEXRS_COMMENT")

    /// A control word (`\relax`, `\csname`) or control symbol (`\\`, `\%`),
    /// stored with its escape character. Catcode 0 starts one.
    @JvmField val CONTROL_SEQUENCE = TexrsTokenType("TEXRS_CONTROL_SEQUENCE")

    /// A control sequence texrs itself implements — the primitives its mouth
    /// and expander carry (`\def`, `\csname`, `\the`, `\ifnum`, `\message`).
    @JvmField val PRIMITIVE = TexrsTokenType("TEXRS_PRIMITIVE")

    /// A control sequence that begins or ends a block (`\begingroup`,
    /// `\endgroup`, `\if…`/`\fi`), which the smart-enter processor closes.
    @JvmField val BLOCK_KEYWORD = TexrsTokenType("TEXRS_BLOCK_KEYWORD")

    /// `{` and `}` (catcodes 1 and 2), split so the brace matcher can pair them.
    @JvmField val BEGIN_GROUP = TexrsTokenType("TEXRS_BEGIN_GROUP")
    @JvmField val END_GROUP = TexrsTokenType("TEXRS_END_GROUP")

    /// `$` (catcode 3) — math shift.
    @JvmField val MATH_SHIFT = TexrsTokenType("TEXRS_MATH_SHIFT")

    /// `&` (catcode 4) — the alignment tab.
    @JvmField val ALIGN_TAB = TexrsTokenType("TEXRS_ALIGN_TAB")

    /// `#1` … `#9` and a bare `#` (catcode 6) — a macro parameter.
    @JvmField val PARAMETER = TexrsTokenType("TEXRS_PARAMETER")

    /// `^` and `_` (catcodes 7 and 8). `^^X` notation is lexed as one token,
    /// since it is one character to the engine.
    @JvmField val SUPERSCRIPT = TexrsTokenType("TEXRS_SUPERSCRIPT")
    @JvmField val SUBSCRIPT = TexrsTokenType("TEXRS_SUBSCRIPT")

    /// `~` and anything else made active (catcode 13).
    @JvmField val ACTIVE_CHAR = TexrsTokenType("TEXRS_ACTIVE_CHAR")

    /// A run of digits, which is what `\count0=12` and `\catcode`\{=1` read.
    @JvmField val NUMBER = TexrsTokenType("TEXRS_NUMBER")

    /// A run of letters (catcode 11) — ordinary text.
    @JvmField val TEXT = TexrsTokenType("TEXRS_TEXT")

    /// Everything else with no special meaning (catcode 12).
    @JvmField val OTHER = TexrsTokenType("TEXRS_OTHER")
}
