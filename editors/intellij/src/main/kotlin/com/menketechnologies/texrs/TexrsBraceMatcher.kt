package com.menketechnologies.texrs

import com.intellij.lang.BracePair
import com.intellij.lang.PairedBraceMatcher
import com.intellij.psi.PsiFile
import com.intellij.psi.tree.IElementType

/**
 * `{` and `}` — the group characters (catcodes 1 and 2). Pairing them gives
 * auto-insertion of the closing brace and the structural highlight when the
 * cursor sits beside one.
 *
 * `$…$` is not paired here: both ends are the same character, which a
 * [PairedBraceMatcher] cannot express, and guessing which one is the opener
 * would highlight the wrong half of the file.
 */
class TexrsBraceMatcher : PairedBraceMatcher {
    private val pairs = arrayOf(
        BracePair(TexrsTokenTypes.BEGIN_GROUP, TexrsTokenTypes.END_GROUP, true),
    )

    override fun getPairs(): Array<BracePair> = pairs

    override fun isPairedBracesAllowedBeforeType(
        lbraceType: IElementType,
        contextType: IElementType?,
    ): Boolean = true

    override fun getCodeConstructStart(file: PsiFile?, openingBraceOffset: Int): Int =
        openingBraceOffset
}
