package com.menketechnologies.texrs

import com.intellij.lang.ASTNode
import com.intellij.lang.ParserDefinition
import com.intellij.lang.PsiBuilder
import com.intellij.lang.PsiParser
import com.intellij.lexer.Lexer
import com.intellij.openapi.fileTypes.FileType
import com.intellij.openapi.project.Project
import com.intellij.psi.FileViewProvider
import com.intellij.psi.PsiElement
import com.intellij.psi.PsiFile
import com.intellij.psi.impl.source.tree.LeafPsiElement
import com.intellij.psi.tree.IFileElementType
import com.intellij.psi.tree.TokenSet

/**
 * A flat parser definition: every lexer token becomes a top-level leaf.
 *
 * The platform needs a `PsiFile` to hang keymap-driven actions on — comment
 * toggling, brace matching, structural selection — and that is all this is for.
 * There is deliberately no tree: TeX has no fixed grammar to build one from,
 * since a document can redefine what any character means, and a parser that
 * pretended otherwise would be wrong in exactly the documents that matter.
 */
class TexrsParserDefinition : ParserDefinition {
    override fun createLexer(project: Project?): Lexer = TexrsLexer()
    override fun createParser(project: Project?): PsiParser = TexrsFlatParser()
    override fun getFileNodeType(): IFileElementType = FILE

    override fun getCommentTokens(): TokenSet = TokenSet.create(TexrsTokenTypes.COMMENT)

    /// TeX has no string literals: a quote is an ordinary character.
    override fun getStringLiteralElements(): TokenSet = TokenSet.EMPTY

    override fun createFile(viewProvider: FileViewProvider): PsiFile = TexrsPsiFile(viewProvider)

    override fun createElement(node: ASTNode): PsiElement = LeafPsiElement(node.elementType, node.text)

    companion object {
        val FILE: IFileElementType = IFileElementType("TEXRS_FILE", TexrsLanguage)
    }
}

private class TexrsFlatParser : PsiParser {
    override fun parse(root: com.intellij.psi.tree.IElementType, builder: PsiBuilder): ASTNode {
        val rootMarker = builder.mark()
        while (!builder.eof()) builder.advanceLexer()
        rootMarker.done(root)
        return builder.treeBuilt
    }
}

class TexrsPsiFile(viewProvider: FileViewProvider) :
    com.intellij.extapi.psi.PsiFileBase(viewProvider, TexrsLanguage) {
    override fun getFileType(): FileType = TexrsFileType
    override fun toString(): String = "TeX File"
}
