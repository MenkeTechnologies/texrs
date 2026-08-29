package com.menketechnologies.texrs

import com.intellij.openapi.editor.DefaultLanguageHighlighterColors as Defaults
import com.intellij.openapi.editor.HighlighterColors
import com.intellij.openapi.editor.colors.TextAttributesKey

/**
 * Plugin-owned [TextAttributesKey]s, one per token category. Each inherits a
 * platform default but lives in its own `TEXRS_*` namespace, so any of them can
 * be rebound under *Settings → Editor → Color Scheme → TeX (texrs)* without
 * touching the rest of the IDE.
 */
object TexrsColors {
    @JvmField val COMMENT = mk("TEXRS_COMMENT", Defaults.LINE_COMMENT)

    /// A control sequence the document defined, or one texrs does not carry.
    @JvmField val CONTROL_SEQUENCE = mk("TEXRS_CONTROL_SEQUENCE", Defaults.IDENTIFIER)

    /// One texrs implements, which is the distinction a reader wants: whether
    /// the engine knows this name or the document has to define it.
    @JvmField val PRIMITIVE = mk("TEXRS_PRIMITIVE", Defaults.KEYWORD)
    @JvmField val BLOCK_KEYWORD = mk("TEXRS_BLOCK_KEYWORD", Defaults.KEYWORD)

    @JvmField val BRACE = mk("TEXRS_BRACE", Defaults.BRACES)
    @JvmField val MATH_SHIFT = mk("TEXRS_MATH_SHIFT", Defaults.NUMBER)
    @JvmField val ALIGN_TAB = mk("TEXRS_ALIGN_TAB", Defaults.OPERATION_SIGN)
    @JvmField val PARAMETER = mk("TEXRS_PARAMETER", Defaults.PARAMETER)
    @JvmField val SCRIPT = mk("TEXRS_SCRIPT", Defaults.OPERATION_SIGN)
    @JvmField val ACTIVE_CHAR = mk("TEXRS_ACTIVE_CHAR", Defaults.PREDEFINED_SYMBOL)
    @JvmField val NUMBER = mk("TEXRS_NUMBER", Defaults.NUMBER)
    @JvmField val TEXT = mk("TEXRS_TEXT", HighlighterColors.TEXT)
    @JvmField val OTHER = mk("TEXRS_OTHER", HighlighterColors.TEXT)
    @JvmField val BAD_CHAR = mk("TEXRS_BAD_CHARACTER", HighlighterColors.BAD_CHARACTER)

    private fun mk(name: String, fallback: TextAttributesKey): TextAttributesKey =
        TextAttributesKey.createTextAttributesKey(name, fallback)
}
