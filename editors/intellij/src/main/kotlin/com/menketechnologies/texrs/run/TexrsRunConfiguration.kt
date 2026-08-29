package com.menketechnologies.texrs.run

import com.intellij.execution.Executor
import com.intellij.execution.configurations.CommandLineState
import com.intellij.execution.configurations.ConfigurationFactory
import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.execution.configurations.LocatableConfigurationBase
import com.intellij.execution.configurations.RunConfiguration
import com.intellij.execution.configurations.RuntimeConfigurationException
import com.intellij.execution.process.KillableColoredProcessHandler
import com.intellij.execution.process.ProcessHandler
import com.intellij.execution.process.ProcessTerminatedListener
import com.intellij.execution.runners.ExecutionEnvironment
import com.intellij.openapi.options.SettingsEditor
import com.intellij.openapi.project.Project
import com.intellij.openapi.util.io.FileUtil
import com.menketechnologies.texrs.TexrsSettings
import java.io.File

class TexrsRunConfiguration(
    project: Project,
    factory: ConfigurationFactory,
    name: String,
) : LocatableConfigurationBase<TexrsRunConfigurationOptions>(project, factory, name) {

    public override fun getOptions(): TexrsRunConfigurationOptions =
        super.getOptions() as TexrsRunConfigurationOptions

    override fun getConfigurationEditor(): SettingsEditor<out RunConfiguration> =
        TexrsRunConfigurationEditor()

    override fun checkConfiguration() {
        val document = options.documentPath.orEmpty()
        if (document.isBlank()) throw RuntimeConfigurationException("Document path is required")
        if (!File(document).isFile) throw RuntimeConfigurationException("Document not found: $document")
        if (options.disasm && options.dumpTokens) {
            // Both stop the pipeline early, at different stages; the binary
            // would honour whichever it sees first, which is not a choice a
            // run configuration should make silently.
            throw RuntimeConfigurationException("--disasm and --dump-tokens cannot both be set")
        }
    }

    override fun getState(executor: Executor, env: ExecutionEnvironment): CommandLineState =
        object : CommandLineState(env) {
            override fun startProcess(): ProcessHandler {
                val settings = TexrsSettings.getInstance()
                val cmd = GeneralCommandLine()
                    .withExePath(settings.executable())
                    .withCharset(Charsets.UTF_8)

                // texrs takes the document as a positional argument, after any
                // flags: `texrs [OPTIONS] FILE.tex`.
                if (options.disasm) cmd.addParameter("--disasm")
                if (options.dumpTokens) cmd.addParameter("--dump-tokens")
                if (options.noCache || settings.passNoCache) cmd.addParameter("--no-cache")
                splitArgs(options.engineArgs.orEmpty()).forEach { cmd.addParameter(it) }
                cmd.addParameter(options.documentPath.orEmpty())

                val workingDirectory = options.workingDirectory?.takeIf { it.isNotBlank() }
                    ?: FileUtil.toSystemDependentName(project.basePath ?: ".")
                cmd.withWorkDirectory(workingDirectory)

                val handler = KillableColoredProcessHandler(cmd)
                ProcessTerminatedListener.attach(handler)
                return handler
            }
        }

    /** Split a command-line string, honouring quotes so a path with a space survives. */
    private fun splitArgs(s: String): List<String> {
        if (s.isBlank()) return emptyList()
        val out = mutableListOf<String>()
        val sb = StringBuilder()
        var quote: Char? = null
        for (c in s) {
            when {
                quote != null && c == quote -> quote = null
                quote != null -> sb.append(c)
                c == '"' || c == '\'' -> quote = c
                c.isWhitespace() -> if (sb.isNotEmpty()) {
                    out += sb.toString()
                    sb.clear()
                }
                else -> sb.append(c)
            }
        }
        if (sb.isNotEmpty()) out += sb.toString()
        return out
    }
}
