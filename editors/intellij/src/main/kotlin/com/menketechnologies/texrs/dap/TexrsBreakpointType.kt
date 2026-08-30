package com.menketechnologies.texrs.dap

import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.xdebugger.breakpoints.XLineBreakpointTypeBase
import com.menketechnologies.texrs.TexrsSettings

/**
 * A line breakpoint in a TeX document.
 *
 * Any line of a supported file can hold one: which lines a run actually reaches
 * is a question about expansion, and a gutter that guessed would be wrong in
 * both directions — a `\def` body's lines are reached only when the macro is
 * called, and a conditional's skipped arm is not reached at all.
 */
class TexrsBreakpointType : XLineBreakpointTypeBase(
    "texrs-line",
    "texrs Line Breakpoint",
    TexrsDebuggerEditorsProvider(),
) {
    override fun canPutAt(file: VirtualFile, line: Int, project: Project): Boolean =
        TexrsSettings.getInstance().isSupportedFile(file.name, file.extension)

    override fun getPriority(): Int = 100
}
