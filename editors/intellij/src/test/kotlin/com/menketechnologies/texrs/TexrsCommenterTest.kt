package com.menketechnologies.texrs

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class TexrsCommenterTest {
    private val commenter = TexrsCommenter()

    @Test
    fun `the line comment is a per-cent sign and a space`() {
        assertEquals("% ", commenter.lineCommentPrefix)
    }

    @Test
    fun `there is no block comment form in TeX`() {
        assertNull(commenter.blockCommentPrefix)
        assertNull(commenter.blockCommentSuffix)
        assertNull(commenter.commentedBlockCommentPrefix)
        assertNull(commenter.commentedBlockCommentSuffix)
    }
}
