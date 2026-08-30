package com.menketechnologies.texrs

import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.components.PersistentStateComponent
import com.intellij.openapi.components.Service
import com.intellij.openapi.components.State
import com.intellij.openapi.components.Storage
import com.intellij.util.xmlb.XmlSerializerUtil

/**
 * Where the texrs binary is, and which files this plugin claims.
 *
 * The binary is one command with three faces — the engine, `--lsp` and `--dap`
 * — so one executable path serves all three, and what is configurable about the
 * server is only how it is started. File extensions are configurable because
 * plenty of TeX lives in `.sty`, `.cls` and `.ltx` files.
 */
@Service(Service.Level.APP)
@State(name = "TexrsSettings", storages = [Storage("texrs.xml")])
class TexrsSettings : PersistentStateComponent<TexrsSettings.State> {
    data class State(
        var texrsExecutable: String? = null,
        var fileExtensions: String = "tex",
        var passNoCache: Boolean = false,
        var lspEnabled: Boolean = true,
        var extraLspArgs: String = "",
        var lspEnv: String = "",
    )

    private var stateData = State()

    override fun getState(): State = stateData
    override fun loadState(state: State) {
        XmlSerializerUtil.copyBean(state, stateData)
    }

    /** A texrs that is not on `PATH`; blank means "find it there". */
    var texrsExecutable: String?
        get() = stateData.texrsExecutable
        set(value) {
            stateData.texrsExecutable = value
        }

    /** Comma / space separated extensions this plugin opens as TeX. */
    var fileExtensions: String
        get() = stateData.fileExtensions
        set(value) {
            stateData.fileExtensions = value
        }

    /** Add `--no-cache` to every run started from the IDE. */
    var passNoCache: Boolean
        get() = stateData.passNoCache
        set(value) {
            stateData.passNoCache = value
        }

    /** Whether to start `texrs --lsp` for a TeX file. */
    var lspEnabled: Boolean
        get() = stateData.lspEnabled
        set(value) {
            stateData.lspEnabled = value
        }

    /** Extra arguments passed after `--lsp`, whitespace separated. */
    var extraLspArgs: String
        get() = stateData.extraLspArgs
        set(value) {
            stateData.extraLspArgs = value
        }

    /** `KEY=VALUE` pairs put in the language server's environment. */
    var lspEnv: String
        get() = stateData.lspEnv
        set(value) {
            stateData.lspEnv = value
        }

    fun supportedExtensions(): List<String> =
        fileExtensions.split(",", " ", ";")
            .map { it.trim().removePrefix(".") }
            .filter { it.isNotEmpty() }

    /** Whether this plugin claims a file, by extension. */
    fun isSupportedFile(filename: String, extension: String?): Boolean {
        if (extension != null && extension in supportedExtensions()) return true
        return filename.substringAfterLast('.', "") in supportedExtensions()
    }

    /** The command to run, falling back to whatever `texrs` is on `PATH`. */
    fun executable(): String = texrsExecutable?.takeIf { it.isNotBlank() } ?: "texrs"

    companion object {
        fun getInstance(): TexrsSettings =
            ApplicationManager.getApplication().getService(TexrsSettings::class.java)
    }
}
