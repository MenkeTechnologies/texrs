package com.menketechnologies.texrs

import com.intellij.lexer.Lexer
import com.intellij.openapi.editor.colors.TextAttributesKey
import com.intellij.openapi.fileTypes.SyntaxHighlighter
import com.intellij.openapi.fileTypes.SyntaxHighlighterBase
import com.intellij.openapi.fileTypes.SyntaxHighlighterFactory
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.psi.TokenType
import com.intellij.psi.tree.IElementType

class TexrsSyntaxHighlighter : SyntaxHighlighterBase() {
    override fun getHighlightingLexer(): Lexer = TexrsLexer()

    override fun getTokenHighlights(type: IElementType): Array<TextAttributesKey> {
        val key: TextAttributesKey? = when (type) {
            TexrsTokenTypes.COMMENT -> TexrsColors.COMMENT
            TexrsTokenTypes.CONTROL_SEQUENCE -> TexrsColors.CONTROL_SEQUENCE
            TexrsTokenTypes.PRIMITIVE -> TexrsColors.PRIMITIVE
            TexrsTokenTypes.BLOCK_KEYWORD -> TexrsColors.BLOCK_KEYWORD
            TexrsTokenTypes.BEGIN_GROUP -> TexrsColors.BRACE
            TexrsTokenTypes.END_GROUP -> TexrsColors.BRACE
            TexrsTokenTypes.MATH_SHIFT -> TexrsColors.MATH_SHIFT
            TexrsTokenTypes.ALIGN_TAB -> TexrsColors.ALIGN_TAB
            TexrsTokenTypes.PARAMETER -> TexrsColors.PARAMETER
            TexrsTokenTypes.SUPERSCRIPT -> TexrsColors.SCRIPT
            TexrsTokenTypes.SUBSCRIPT -> TexrsColors.SCRIPT
            TexrsTokenTypes.ACTIVE_CHAR -> TexrsColors.ACTIVE_CHAR
            TexrsTokenTypes.NUMBER -> TexrsColors.NUMBER
            TexrsTokenTypes.TEXT -> TexrsColors.TEXT
            TexrsTokenTypes.OTHER -> TexrsColors.OTHER
            TokenType.BAD_CHARACTER -> TexrsColors.BAD_CHAR
            else -> null
        }
        return if (key == null) emptyArray() else arrayOf(key)
    }
}

class TexrsSyntaxHighlighterFactory : SyntaxHighlighterFactory() {
    override fun getSyntaxHighlighter(project: Project?, virtualFile: VirtualFile?): SyntaxHighlighter =
        TexrsSyntaxHighlighter()
}
