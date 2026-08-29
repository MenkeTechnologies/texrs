package com.menketechnologies.texrs.actions

import com.intellij.ide.actions.CreateFileFromTemplateAction
import com.intellij.ide.actions.CreateFileFromTemplateDialog
import com.intellij.openapi.project.Project
import com.intellij.psi.PsiDirectory
import com.intellij.psi.PsiFile
import com.intellij.psi.PsiFileFactory
import com.menketechnologies.texrs.TexrsFileType
import com.menketechnologies.texrs.TexrsIcons

/**
 * File > New > TeX Document.
 *
 * Every template opens by giving `{` and `}` their category codes, because
 * INITEX does not: a bare `tex` run treats them as ordinary characters, and a
 * new file that omitted the line would fail on its first group with `Missing {
 * inserted`. Templates are inline rather than `fileTemplates/` resources so the
 * plugin stays a single jar with nothing to extract at runtime.
 */
class CreateTexrsFileAction :
    CreateFileFromTemplateAction("TeX Document", "Create a new TeX document", TexrsIcons.FILE) {

    override fun getActionName(directory: PsiDirectory?, newName: String, templateName: String?): String =
        "Create TeX Document"

    override fun buildDialog(
        project: Project,
        directory: PsiDirectory,
        builder: CreateFileFromTemplateDialog.Builder,
    ) {
        builder
            .setTitle("New TeX Document")
            .addKind("Document (catcodes + \\message)", TexrsIcons.FILE, TPL_DOCUMENT)
            .addKind("Macro file (\\def with parameters)", TexrsIcons.FILE, TPL_MACROS)
            .addKind("Empty", TexrsIcons.FILE, TPL_EMPTY)
    }

    override fun createFile(name: String, templateName: String, dir: PsiDirectory): PsiFile? {
        val fileName = if (name.contains('.')) name else "$name.tex"
        val body = when (templateName) {
            TPL_DOCUMENT -> DOCUMENT_BODY
            TPL_MACROS -> MACROS_BODY
            else -> ""
        }
        val file = PsiFileFactory.getInstance(dir.project)
            .createFileFromText(fileName, TexrsFileType, body)
        return dir.add(file) as? PsiFile
    }

    companion object {
        private const val TPL_DOCUMENT = "Document"
        private const val TPL_MACROS = "Macros"
        private const val TPL_EMPTY = "Empty"

        private val DOCUMENT_BODY = """
            |% INITEX leaves `{` and `}` ordinary characters, so a document that
            |% wants groups says so first -- exactly as plain.tex does.
            |\catcode`\{=1 \catcode`\}=2
            |
            |\message{hello from texrs}
            |\end
            |""".trimMargin()

        private val MACROS_BODY = """
            |% A macro file: definitions only, no \end, meant to be read by a
            |% document. `#1` is an undelimited parameter; `(#1,#2)` delimits.
            |\catcode`\{=1 \catcode`\}=2 \catcode`\#=6
            |
            |\def\greet#1{\message{hello, #1}}
            |\def\pair(#1,#2){\message{#1 and #2}}
            |""".trimMargin()
    }
}
