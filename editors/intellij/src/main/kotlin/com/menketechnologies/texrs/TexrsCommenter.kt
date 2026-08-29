package com.menketechnologies.texrs

import com.intellij.lang.Commenter

/**
 * TeX comments with `%` (catcode 14), which runs to the end of the line — and
 * takes the line ending with it, which is why a commented line joins the next
 * one rather than leaving a space.
 *
 * There is no block-comment form in TeX itself, so those hooks return null and
 * Cmd+Opt+/ falls back to commenting line by line.
 */
class TexrsCommenter : Commenter {
    override fun getLineCommentPrefix(): String = "% "
    override fun getBlockCommentPrefix(): String? = null
    override fun getBlockCommentSuffix(): String? = null
    override fun getCommentedBlockCommentPrefix(): String? = null
    override fun getCommentedBlockCommentSuffix(): String? = null
}
