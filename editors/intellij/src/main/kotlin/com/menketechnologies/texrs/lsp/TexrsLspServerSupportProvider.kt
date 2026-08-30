package com.menketechnologies.texrs.lsp

import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.platform.lsp.api.LspServerSupportProvider
import com.intellij.platform.lsp.api.LspServerSupportProvider.LspServerStarter
import com.menketechnologies.texrs.TexrsSettings

class TexrsLspServerSupportProvider : LspServerSupportProvider {
    override fun fileOpened(project: Project, file: VirtualFile, serverStarter: LspServerStarter) {
        val settings = TexrsSettings.getInstance()
        if (!settings.lspEnabled) return
        if (!settings.isSupportedFile(file.name, file.extension)) return
        serverStarter.ensureServerStarted(TexrsLspServerDescriptor(project))
    }
}
