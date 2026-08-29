package com.menketechnologies.texrs

import com.intellij.psi.PsiElement
import com.intellij.spellchecker.tokenizer.SpellcheckingStrategy
import com.intellij.spellchecker.tokenizer.Tokenizer

/**
 * Spell-check the prose and leave the markup alone.
 *
 * A TeX file is mostly text a dictionary should check, but control sequences
 * are not words — `\csname`, `\hbox`, `\ifnum` and every macro a document
 * defines would be flagged, which trains a reader to ignore the inspection
 * entirely. Comments and text are checked; everything else is skipped.
 */
class TexrsSpellcheckingStrategy : SpellcheckingStrategy() {
    override fun getTokenizer(element: PsiElement?): Tokenizer<*> {
        val type = element?.node?.elementType
        return when (type) {
            TexrsTokenTypes.TEXT, TexrsTokenTypes.COMMENT -> super.getTokenizer(element)
            else -> EMPTY_TOKENIZER
        }
    }
}
