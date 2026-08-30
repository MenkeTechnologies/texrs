package com.menketechnologies.texrs.dap

import com.intellij.openapi.editor.Document
import com.intellij.openapi.fileTypes.FileType
import com.intellij.openapi.project.Project
import com.intellij.psi.PsiDocumentManager
import com.intellij.psi.PsiFileFactory
import com.intellij.xdebugger.XExpression
import com.intellij.xdebugger.XSourcePosition
import com.intellij.xdebugger.evaluation.EvaluationMode
import com.intellij.xdebugger.evaluation.XDebuggerEditorsProvider
import com.menketechnologies.texrs.TexrsFileType

/// The editor behind Evaluate Expression and a conditional breakpoint: a TeX
/// buffer, so what is typed there is highlighted and completed like the document.
class TexrsDebuggerEditorsProvider : XDebuggerEditorsProvider() {
    override fun getFileType(): FileType = TexrsFileType

    override fun createDocument(
        project: Project,
        expression: XExpression,
        sourcePosition: XSourcePosition?,
        mode: EvaluationMode,
    ): Document {
        val psi = PsiFileFactory.getInstance(project).createFileFromText(
            "_texrs_expr.tex",
            TexrsFileType,
            expression.expression,
        )
        return PsiDocumentManager.getInstance(project).getDocument(psi)
            ?: com.intellij.openapi.editor.EditorFactory.getInstance()
                .createDocument(expression.expression)
    }
}
