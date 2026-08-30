package com.menketechnologies.texrs.dap

import com.google.gson.JsonArray
import com.google.gson.JsonObject
import com.intellij.openapi.application.ReadAction
import com.intellij.xdebugger.XDebuggerManager
import com.intellij.xdebugger.breakpoints.XBreakpointHandler
import com.intellij.xdebugger.breakpoints.XBreakpointProperties
import com.intellij.xdebugger.breakpoints.XLineBreakpoint

/**
 * Keeps the adapter's idea of a file's breakpoints equal to the IDE's.
 *
 * DAP's `setBreakpoints` replaces every breakpoint in a file rather than adding
 * one, so both adding and removing send the whole set for that file — sending
 * only the new one would silently clear the others.
 */
class TexrsBreakpointHandler(
    private val process: TexrsDebugProcess,
) : XBreakpointHandler<XLineBreakpoint<XBreakpointProperties<*>>>(TexrsBreakpointType::class.java) {

    override fun registerBreakpoint(bp: XLineBreakpoint<XBreakpointProperties<*>>) {
        resync(bp.fileUrl)
    }

    override fun unregisterBreakpoint(
        bp: XLineBreakpoint<XBreakpointProperties<*>>,
        temporary: Boolean,
    ) {
        resync(bp.fileUrl)
    }

    private fun resync(fileUrl: String) {
        val client = process.client ?: return
        val path = fileUrl.removePrefix("file://")
        val lines: List<Int> = ReadAction.compute<List<Int>, RuntimeException> {
            XDebuggerManager.getInstance(process.session.project)
                .breakpointManager
                .getBreakpoints(TexrsBreakpointType::class.java)
                .filter { it.fileUrl == fileUrl && it.isEnabled }
                // The adapter counts lines from one.
                .map { it.line + 1 }
        }
        client.requestAsync("setBreakpoints", breakpointArgs(path, lines))
    }

    companion object {
        /// `setBreakpoints` arguments for one file.
        fun breakpointArgs(path: String, lines: List<Int>): JsonObject = JsonObject().apply {
            add("source", JsonObject().apply { addProperty("path", path) })
            val arr = JsonArray()
            for (line in lines) {
                arr.add(JsonObject().apply { addProperty("line", line) })
            }
            add("breakpoints", arr)
        }
    }
}
