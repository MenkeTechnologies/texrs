package com.menketechnologies.texrs.lsp

import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.openapi.application.PathManager
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.project.Project
import com.intellij.openapi.util.SystemInfo
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.platform.lsp.api.ProjectWideLspServerDescriptor
import com.intellij.platform.lsp.api.customization.LspDiagnosticsSupport
import com.menketechnologies.texrs.TexrsSettings
import java.io.File

/**
 * The client for `texrs --lsp`.
 *
 * What is opted into here is exactly what the server advertises in
 * `src/lsp.rs`: full-text sync, completion over the primitive corpus (with `\`
 * as the trigger character, since that is what opens a control sequence), hover,
 * and diagnostics. Nothing else is enabled — an opt-in for a capability the
 * server does not have is a feature that silently does nothing, which is worse
 * than a missing one because it looks present.
 */
class TexrsLspServerDescriptor(project: Project) :
    ProjectWideLspServerDescriptor(project, "texrs") {

    override fun isSupportedFile(file: VirtualFile): Boolean =
        TexrsSettings.getInstance().isSupportedFile(file.name, file.extension)

    /// The server publishes diagnostics; the platform renders them.
    override val lspDiagnosticsSupport: LspDiagnosticsSupport = LspDiagnosticsSupport()

    /// Hover is advertised (`hover_provider`), and carries the primitive's
    /// description out of the corpus.
    override val lspHoverSupport: Boolean = true

    /// Go-to-definition is NOT advertised by the server, so it stays off: with
    /// it on, Cmd-B would route through the LSP and come back empty rather than
    /// falling through to what the platform would otherwise do.
    override val lspGoToDefinitionSupport: Boolean = false

    override fun createCommandLine(): GeneralCommandLine {
        val settings = TexrsSettings.getInstance()
        val exe = resolveExe()
        LOG.info("starting the texrs language server: $exe --lsp ${settings.extraLspArgs}")
        val cmd = GeneralCommandLine(exe)
            .withParameters("--lsp")
            .withWorkDirectory(project.basePath ?: PathManager.getHomePath())
            .withEnvironment("RUST_BACKTRACE", "1")
        splitArgs(settings.extraLspArgs).forEach { cmd.addParameter(it) }
        for (pair in splitArgs(settings.lspEnv)) {
            val at = pair.indexOf('=')
            if (at > 0) cmd.withEnvironment(pair.substring(0, at), pair.substring(at + 1))
        }
        return cmd
    }

    /// The configured binary if it is one, else whatever `texrs` is on PATH.
    /// Resolving PATH here rather than letting the process inherit it is what
    /// makes the failure legible: an IDE launched from the Dock has a different
    /// PATH from a shell, and "cannot run program" says less than a path does.
    private fun resolveExe(): String {
        val settings = TexrsSettings.getInstance()
        settings.texrsExecutable
            ?.takeIf { it.isNotBlank() && File(it).canExecute() }
            ?.let { return it }
        return findOnPath("texrs") ?: "texrs"
    }

    private fun findOnPath(name: String): String? {
        val pathEnv = System.getenv("PATH") ?: return null
        val suffixes = if (SystemInfo.isWindows) listOf(".exe", ".bat", ".cmd", "") else listOf("")
        for (dir in pathEnv.split(File.pathSeparator)) {
            for (suffix in suffixes) {
                val candidate = File(dir, name + suffix)
                if (candidate.canExecute()) return candidate.absolutePath
            }
        }
        return null
    }

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

    companion object {
        private val LOG = Logger.getInstance(TexrsLspServerDescriptor::class.java)
    }
}
