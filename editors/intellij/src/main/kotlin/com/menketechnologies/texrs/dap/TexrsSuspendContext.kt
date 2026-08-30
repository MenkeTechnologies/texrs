package com.menketechnologies.texrs.dap

import com.intellij.xdebugger.frame.XExecutionStack
import com.intellij.xdebugger.frame.XStackFrame
import com.intellij.xdebugger.frame.XSuspendContext

class TexrsSuspendContext(private val stack: TexrsExecutionStack) : XSuspendContext() {
    override fun getActiveExecutionStack(): XExecutionStack = stack
}

/// One stack. texrs runs a document on a single thread, so there is never a
/// second one to switch to.
class TexrsExecutionStack : XExecutionStack("main") {

    @Volatile
    private var frames: List<TexrsStackFrame> = emptyList()

    fun setFrames(newFrames: List<TexrsStackFrame>) {
        frames = newFrames
    }

    override fun getTopFrame(): XStackFrame? = frames.firstOrNull()

    override fun computeStackFrames(firstFrameIndex: Int, container: XStackFrameContainer) {
        val slice = if (firstFrameIndex <= 0) frames else frames.drop(firstFrameIndex)
        container.addStackFrames(slice, true)
    }
}
