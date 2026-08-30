package com.menketechnologies.texrs.run

import com.intellij.execution.ExecutionException
import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.execution.configurations.RunProfile
import com.intellij.execution.configurations.RunProfileState
import com.intellij.execution.executors.DefaultDebugExecutor
import com.intellij.execution.process.ProcessHandler
import com.intellij.execution.process.ProcessOutputTypes
import com.intellij.execution.runners.DefaultProgramRunner
import com.intellij.execution.runners.ExecutionEnvironment
import com.intellij.execution.ui.RunContentDescriptor
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.util.io.FileUtil
import com.intellij.xdebugger.XDebugProcess
import com.intellij.xdebugger.XDebugProcessStarter
import com.intellij.xdebugger.XDebugSession
import com.intellij.xdebugger.XDebuggerManager
import com.menketechnologies.texrs.TexrsSettings
import com.menketechnologies.texrs.dap.TexrsDebugProcess
import java.io.OutputStream

/**
 * The Debug button: spawns `texrs --dap` and drives it over the launched
 * process's stdio.
 *
 * The adapter is stdio-only — there is no port to connect to — so the process's
 * stdout carries protocol frames and nothing else. The handler below therefore
 * pumps stderr to the console and deliberately leaves stdout alone: decoding it
 * here would take frames away from the DAP client and print raw JSON at the
 * user. The document's own output reaches the console as DAP `output` events.
 */
class TexrsDebugRunner : DefaultProgramRunner() {
    override fun getRunnerId(): String = "TexrsDebugRunner"

    override fun canRun(executorId: String, profile: RunProfile): Boolean =
        executorId == DefaultDebugExecutor.EXECUTOR_ID && profile is TexrsRunConfiguration

    @Throws(ExecutionException::class)
    override fun doExecute(state: RunProfileState, env: ExecutionEnvironment): RunContentDescriptor? {
        val cfg = env.runProfile as TexrsRunConfiguration
        val cmd = GeneralCommandLine()
            .withExePath(TexrsSettings.getInstance().executable())
            .withCharset(Charsets.UTF_8)
            .withParameters("--dap")
        val workingDirectory = cfg.options.workingDirectory?.takeIf { it.isNotBlank() }
            ?: FileUtil.toSystemDependentName(env.project.basePath ?: ".")
        cmd.withWorkDirectory(workingDirectory)

        val process: Process = cmd.createProcess()
        val handler = TexrsDapProcessHandler(process)

        val session: XDebugSession = XDebuggerManager.getInstance(env.project).startSession(
            env,
            object : XDebugProcessStarter() {
                override fun start(session: XDebugSession): XDebugProcess = TexrsDebugProcess(
                    session = session,
                    processHandler = handler,
                    // The adapter's stdout is the protocol; its stdin takes ours.
                    dapInput = process.inputStream,
                    dapOutput = process.outputStream,
                    documentPath = cfg.options.documentPath.orEmpty(),
                    workingDirectory = workingDirectory,
                )
            },
        )

        return descriptorWithoutSplitDebuggerWarning(session)
            ?: @Suppress("DEPRECATION") session.runContentDescriptor
    }

    /**
     * The non-deprecated descriptor when this platform build has it.
     *
     * `getRunContentDescriptor` is deprecated in favour of a method that only
     * exists in newer builds, and calling the old one logs a split-debugger
     * warning on every launch. Reflection keeps both builds working; the
     * fallback is the deprecated call, which still returns the right thing.
     */
    private fun descriptorWithoutSplitDebuggerWarning(session: XDebugSession): RunContentDescriptor? =
        try {
            session.javaClass.methods
                .firstOrNull {
                    it.name == "getMockRunContentDescriptorIfInitialized" && it.parameterCount == 0
                }
                ?.also { it.isAccessible = true }
                ?.invoke(session) as? RunContentDescriptor
        } catch (e: Throwable) {
            LOG.debug("could not find the newer descriptor accessor", e)
            null
        }

    companion object {
        private val LOG = Logger.getInstance(TexrsDebugRunner::class.java)
    }
}

/**
 * Lifecycle only: reports the adapter's stderr and its exit, and never reads
 * stdout — see the class comment above.
 */
private class TexrsDapProcessHandler(private val process: Process) : ProcessHandler() {
    init {
        Thread({
            try {
                process.errorStream.bufferedReader().forEachLine {
                    notifyTextAvailable(it + "\n", ProcessOutputTypes.STDERR)
                }
            } catch (_: Exception) {
                // The stream closes when the adapter exits.
            }
        }, "texrs-dap-stderr").apply {
            isDaemon = true
            start()
        }

        Thread({
            try {
                notifyProcessTerminated(process.waitFor())
            } catch (_: InterruptedException) {
                // Shutting down.
            }
        }, "texrs-dap-waiter").apply {
            isDaemon = true
            start()
        }
    }

    override fun destroyProcessImpl() {
        // The waiter thread reports termination once it exits.
        process.destroy()
    }

    override fun detachProcessImpl() {
        notifyProcessDetached()
    }

    override fun detachIsDefault(): Boolean = false

    override fun getProcessInput(): OutputStream? = null
}
