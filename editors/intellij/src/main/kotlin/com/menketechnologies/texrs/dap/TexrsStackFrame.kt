package com.menketechnologies.texrs.dap

import com.intellij.openapi.vfs.LocalFileSystem
import com.intellij.ui.ColoredTextContainer
import com.intellij.ui.SimpleTextAttributes
import com.intellij.xdebugger.XDebuggerUtil
import com.intellij.xdebugger.XSourcePosition
import com.intellij.xdebugger.evaluation.XDebuggerEvaluator
import com.intellij.xdebugger.frame.XCompositeNode
import com.intellij.xdebugger.frame.XStackFrame
import com.intellij.xdebugger.frame.XValueChildrenList

class TexrsStackFrame(
    private val client: TexrsDapClient?,
    private val frameId: Int,
    private val name: String,
    private val file: String,
    private val line: Int,
    private val children: List<TexrsValue>,
) : XStackFrame() {

    override fun getSourcePosition(): XSourcePosition? {
        if (file.isBlank()) return null
        val vf = LocalFileSystem.getInstance().refreshAndFindFileByPath(file) ?: return null
        // The adapter counts lines from one, as TeX does; the platform counts
        // from zero.
        return XDebuggerUtil.getInstance().createPosition(vf, (line - 1).coerceAtLeast(0))
    }

    override fun computeChildren(node: XCompositeNode) {
        val list = XValueChildrenList()
        for (child in children) list.add(child)
        node.addChildren(list, true)
    }

    override fun getEvaluator(): XDebuggerEvaluator = TexrsEvaluator(client, frameId)

    override fun customizePresentation(component: ColoredTextContainer) {
        val where = "${file.substringAfterLast('/').ifBlank { "<unknown>" }}:$line"
        val label = if (name.isBlank()) "frame $frameId ($where)" else "$name ($where)"
        component.append(label, SimpleTextAttributes.REGULAR_ATTRIBUTES)
    }
}
