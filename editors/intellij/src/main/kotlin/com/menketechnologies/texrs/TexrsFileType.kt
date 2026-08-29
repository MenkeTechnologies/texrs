package com.menketechnologies.texrs

import com.intellij.openapi.fileTypes.LanguageFileType
import javax.swing.Icon

object TexrsFileType : LanguageFileType(TexrsLanguage) {
    override fun getName(): String = "TeX"
    override fun getDescription(): String = "TeX document (texrs)"
    override fun getDefaultExtension(): String = "tex"
    override fun getIcon(): Icon = TexrsIcons.FILE
}
