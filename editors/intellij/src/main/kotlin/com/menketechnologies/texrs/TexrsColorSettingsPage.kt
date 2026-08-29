package com.menketechnologies.texrs

import com.intellij.openapi.editor.colors.TextAttributesKey
import com.intellij.openapi.fileTypes.SyntaxHighlighter
import com.intellij.openapi.options.colors.AttributesDescriptor
import com.intellij.openapi.options.colors.ColorDescriptor
import com.intellij.openapi.options.colors.ColorSettingsPage
import javax.swing.Icon

class TexrsColorSettingsPage : ColorSettingsPage {
    private val attrs = arrayOf(
        AttributesDescriptor("Comments//Line comment (%)", TexrsColors.COMMENT),

        AttributesDescriptor("Control sequences//Defined by the document", TexrsColors.CONTROL_SEQUENCE),
        AttributesDescriptor("Control sequences//Primitive texrs implements", TexrsColors.PRIMITIVE),
        AttributesDescriptor("Control sequences//Conditional / group", TexrsColors.BLOCK_KEYWORD),

        AttributesDescriptor("Category codes//Group braces { }", TexrsColors.BRACE),
        AttributesDescriptor("Category codes//Math shift (\$)", TexrsColors.MATH_SHIFT),
        AttributesDescriptor("Category codes//Alignment tab (&)", TexrsColors.ALIGN_TAB),
        AttributesDescriptor("Category codes//Parameter (#1)", TexrsColors.PARAMETER),
        AttributesDescriptor("Category codes//Superscript / subscript (^ _)", TexrsColors.SCRIPT),
        AttributesDescriptor("Category codes//Active character (~)", TexrsColors.ACTIVE_CHAR),

        AttributesDescriptor("Text//Number", TexrsColors.NUMBER),
        AttributesDescriptor("Text//Letters", TexrsColors.TEXT),
        AttributesDescriptor("Text//Other characters", TexrsColors.OTHER),

        AttributesDescriptor("Errors//Bad character", TexrsColors.BAD_CHAR),
    )

    override fun getIcon(): Icon = TexrsIcons.FILE
    override fun getHighlighter(): SyntaxHighlighter = TexrsSyntaxHighlighter()
    override fun getDemoText(): String = DEMO
    override fun getAdditionalHighlightingTagToDescriptorMap(): MutableMap<String, TextAttributesKey>? = null
    override fun getAttributeDescriptors(): Array<AttributesDescriptor> = attrs
    override fun getColorDescriptors(): Array<ColorDescriptor> = ColorDescriptor.EMPTY_ARRAY
    override fun getDisplayName(): String = "TeX (texrs)"

    companion object {
        // Every category appears at least once, so each slot has a live preview
        // in Settings -> Editor -> Color Scheme. The document is a real one:
        // it opens by giving `{` and `}` their catcodes, because INITEX does
        // not, which is the first thing a texrs document has to do.
        private val DEMO = """
            % demo.tex -- every token category, for colour tweaking.
            % A per-cent sign comments to the end of the line, and takes the
            % line ending with it.

            % ---- category codes come first: INITEX leaves these ordinary ----
            \catcode`\{=1 \catcode`\}=2 \catcode`\#=6 \catcode`\~=13

            % ---- macros, with delimited and undelimited parameters ----
            \def\greet#1{\message{hello, #1}}
            \def\pair(#1,#2){\message{#1 and #2}}
            \edef\now{\the\count0}

            % ---- registers and arithmetic ----
            \count0=12
            \advance\count0 by 30
            \multiply\count0 by 2

            % ---- conditionals ----
            \ifnum\count0>40
                \message{big}
            \else
                \message{small}
            \fi

            % ---- \csname builds a name from what expands ----
            \expandafter\def\csname greet:formal\endcsname{\message{good day}}

            % ---- groups keep a definition local ----
            \begingroup
                \def\greet#1{\message{HI #1}}
                \greet{world}
            \endgroup

            \greet{world}
            \message{\string\greet\space is \meaning}
            \end
        """.trimIndent()
    }
}
