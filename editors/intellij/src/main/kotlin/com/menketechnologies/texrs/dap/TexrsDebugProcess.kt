package com.menketechnologies.texrs.dap

import com.google.gson.JsonArray
import com.google.gson.JsonObject
import com.intellij.execution.filters.TextConsoleBuilderFactory
import com.intellij.execution.process.ProcessHandler
import com.intellij.execution.process.ProcessOutputTypes
import com.intellij.execution.ui.ConsoleView
import com.intellij.execution.ui.ExecutionConsole
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.diagnostic.Logger
import com.intellij.xdebugger.XDebugProcess
import com.intellij.xdebugger.XDebugSession
import com.intellij.xdebugger.XDebuggerManager
import com.intellij.xdebugger.XSourcePosition
import com.intellij.xdebugger.breakpoints.XBreakpointHandler
import com.intellij.xdebugger.evaluation.XDebuggerEditorsProvider
import com.intellij.xdebugger.frame.XSuspendContext
import java.io.InputStream
import java.io.OutputStream

/**
 * The debug session, speaking DAP to `texrs --dap` over its stdio.
 *
 * The document's own `\message` output arrives as DAP `output` events on the
 * same stream that carries the protocol, so the console is fed from those
 * events rather than from the process's stdout — which is the protocol stream,
 * and reading it twice would take frames away from the client.
 */
class TexrsDebugProcess(
    session: XDebugSession,
    private val processHandler: ProcessHandler,
    private val dapInput: InputStream,
    private val dapOutput: OutputStream,
    private val documentPath: String,
    private val workingDirectory: String?,
) : XDebugProcess(session) {

    @Volatile
    var client: TexrsDapClient? = null
        private set

    private val executionStack = TexrsExecutionStack()
    private val editorsProvider = TexrsDebuggerEditorsProvider()
    private val breakpointHandlers = arrayOf<XBreakpointHandler<*>>(TexrsBreakpointHandler(this))

    override fun getEditorsProvider(): XDebuggerEditorsProvider = editorsProvider
    override fun getBreakpointHandlers(): Array<XBreakpointHandler<*>> = breakpointHandlers
    override fun doGetProcessHandler(): ProcessHandler = processHandler

    override fun createConsole(): ExecutionConsole {
        val console = TextConsoleBuilderFactory.getInstance()
            .createBuilder(session.project)
            .console as ConsoleView
        console.attachToProcess(processHandler)
        return console
    }

    override fun sessionInitialized() {
        super.sessionInitialized()
        ApplicationManager.getApplication().invokeLater {
            if (!processHandler.isStartNotified) {
                processHandler.startNotify()
            }
        }

        val c = TexrsDapClient(
            output = dapOutput,
            input = dapInput,
            onEvent = { event, body -> handleEvent(event, body) },
        )
        client = c

        // The handshake blocks on the adapter's answers, so it runs off the UI
        // thread: an adapter that never replies must not freeze the IDE.
        ApplicationManager.getApplication().executeOnPooledThread {
            try {
                c.request("initialize", initializeArgs())
                sendAllBreakpoints()
                c.request("configurationDone")
                c.request("launch", launchArgs())
            } catch (t: Throwable) {
                LOG.warn("the debug adapter handshake failed", t)
            }
        }
    }

    private fun initializeArgs(): JsonObject = JsonObject().apply {
        addProperty("clientID", "intellij-texrs")
        addProperty("clientName", "IntelliJ texrs")
        addProperty("adapterID", "texrs")
        addProperty("locale", "en-US")
        // TeX counts lines and columns from one, and so does the adapter.
        addProperty("linesStartAt1", true)
        addProperty("columnsStartAt1", true)
        addProperty("pathFormat", "path")
        addProperty("supportsVariableType", true)
        addProperty("supportsRunInTerminalRequest", false)
        addProperty("supportsProgressReporting", false)
    }

    private fun launchArgs(): JsonObject = JsonObject().apply {
        addProperty("program", documentPath)
        addProperty("stopOnEntry", false)
        workingDirectory?.let { addProperty("cwd", it) }
    }

    /// Every breakpoint the IDE holds, grouped by file, before the run starts.
    private fun sendAllBreakpoints() {
        val c = client ?: return
        val byFile = mutableMapOf<String, MutableList<Int>>()
        val manager = XDebuggerManager.getInstance(session.project).breakpointManager
        for (bp in manager.getBreakpoints(TexrsBreakpointType::class.java)) {
            if (!bp.isEnabled) continue
            val path = bp.fileUrl.removePrefix("file://")
            byFile.getOrPut(path) { mutableListOf() }.add(bp.line + 1)
        }
        for ((path, lines) in byFile) {
            c.requestAsync("setBreakpoints", TexrsBreakpointHandler.breakpointArgs(path, lines))
        }
    }

    private fun handleEvent(event: String, body: JsonObject) {
        when (event) {
            "stopped" -> onStopped()
            "terminated", "exited" -> session.stop()
            "output" -> {
                val text = body.get("output")?.asString ?: return
                val stream = when (body.get("category")?.asString) {
                    "stderr" -> ProcessOutputTypes.STDERR
                    "console" -> ProcessOutputTypes.SYSTEM
                    else -> ProcessOutputTypes.STDOUT
                }
                processHandler.notifyTextAvailable(text, stream)
            }
            else -> {}
        }
    }

    /// Build the frames and their variables, then tell the session where it is.
    private fun onStopped() {
        ApplicationManager.getApplication().executeOnPooledThread {
            try {
                val c = client ?: return@executeOnPooledThread
                val trace = c.request(
                    "stackTrace",
                    JsonObject().apply {
                        addProperty("threadId", 1)
                        addProperty("startFrame", 0)
                        addProperty("levels", 100)
                    },
                ) ?: return@executeOnPooledThread
                val rawFrames: JsonArray =
                    trace.getAsJsonArray("stackFrames") ?: return@executeOnPooledThread
                if (rawFrames.isEmpty) return@executeOnPooledThread

                val frames = rawFrames.map { raw ->
                    val frame = raw.asJsonObject
                    val frameId = frame.get("id")?.asInt ?: 0
                    TexrsStackFrame(
                        client = c,
                        frameId = frameId,
                        name = frame.get("name")?.asString ?: "<frame>",
                        file = frame.getAsJsonObject("source")?.get("path")?.asString ?: "",
                        line = frame.get("line")?.asInt ?: 0,
                        children = variablesOf(c, frameId),
                    )
                }
                executionStack.setFrames(frames)
                val context = TexrsSuspendContext(executionStack)
                ApplicationManager.getApplication().invokeLater {
                    session.positionReached(context)
                }
            } catch (t: Throwable) {
                LOG.warn("could not read the stopped state", t)
            }
        }
    }

    /// Every variable in every scope of one frame, flattened: TeX's scopes are
    /// the macro table and the registers, and both belong beside the frame.
    private fun variablesOf(c: TexrsDapClient, frameId: Int): List<TexrsValue> {
        val scopes = c.request("scopes", JsonObject().apply { addProperty("frameId", frameId) })
            ?.getAsJsonArray("scopes")
            ?: return emptyList()
        val out = mutableListOf<TexrsValue>()
        for (scope in scopes) {
            val varRef = scope.asJsonObject.get("variablesReference")?.asInt ?: continue
            if (varRef == 0) continue
            val body = c.request(
                "variables",
                JsonObject().apply { addProperty("variablesReference", varRef) },
            ) ?: continue
            body.getAsJsonArray("variables")?.forEach { entry ->
                val v = entry.asJsonObject
                out += TexrsValue(
                    name = v.get("name")?.asString ?: "?",
                    repr = v.get("value")?.asString ?: "",
                    kind = v.get("type")?.asString ?: "",
                    varRef = v.get("variablesReference")?.asInt ?: 0,
                    client = c,
                )
            }
        }
        return out
    }

    private fun step(command: String) {
        client?.requestAsync(command, JsonObject().apply { addProperty("threadId", 1) })
    }

    override fun resume(context: XSuspendContext?) = step("continue")
    override fun startStepOver(context: XSuspendContext?) = step("next")
    override fun startStepInto(context: XSuspendContext?) = step("stepIn")
    override fun startStepOut(context: XSuspendContext?) = step("stepOut")
    override fun startPausing() = step("pause")

    override fun stop() {
        client?.requestAsync(
            "disconnect",
            JsonObject().apply { addProperty("terminateDebuggee", true) },
        )
        client?.close()
        try {
            dapInput.close()
        } catch (_: Exception) {
        }
        if (!processHandler.isProcessTerminated) {
            try {
                processHandler.destroyProcess()
            } catch (_: Exception) {
            }
        }
    }

    override fun runToPosition(position: XSourcePosition, context: XSuspendContext?) {
        val c = client ?: return
        // A one-shot breakpoint at the target, then let it run. DAP has no
        // "run to here", and this is what the protocol gives instead.
        c.requestAsync(
            "setBreakpoints",
            TexrsBreakpointHandler.breakpointArgs(position.file.path, listOf(position.line + 1)),
        )
        step("continue")
    }

    companion object {
        private val LOG = Logger.getInstance(TexrsDebugProcess::class.java)
    }
}
