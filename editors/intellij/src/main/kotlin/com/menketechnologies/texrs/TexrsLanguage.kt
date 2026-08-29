package com.menketechnologies.texrs

import com.intellij.lang.Language

object TexrsLanguage : Language("texrs") {
    private fun readResolve(): Any = TexrsLanguage
    override fun getDisplayName(): String = "TeX (texrs)"

    /// TeX is case-sensitive: `\Relax` and `\relax` are different control
    /// sequences, and a control word ends at the first non-letter.
    override fun isCaseSensitive(): Boolean = true
}
