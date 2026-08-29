package com.menketechnologies.texrs.run

import com.intellij.execution.actions.ConfigurationContext
import com.intellij.execution.actions.LazyRunConfigurationProducer
import com.intellij.execution.configurations.ConfigurationFactory
import com.intellij.openapi.util.Ref
import com.intellij.psi.PsiElement
import com.intellij.psi.PsiFile
import com.menketechnologies.texrs.TexrsSettings

/// Makes the gutter and context-menu Run appear on any TeX document, with the
/// configuration filled in from the file it was invoked on.
class TexrsRunConfigurationProducer : LazyRunConfigurationProducer<TexrsRunConfiguration>() {

    override fun getConfigurationFactory(): ConfigurationFactory =
        TexrsRunConfigurationType.getInstance().factory

    override fun setupConfigurationFromContext(
        config: TexrsRunConfiguration,
        context: ConfigurationContext,
        sourceElement: Ref<PsiElement>,
    ): Boolean {
        val file: PsiFile = context.psiLocation?.containingFile ?: return false
        val vf = file.virtualFile ?: return false
        if (!TexrsSettings.getInstance().isSupportedFile(vf.name, vf.extension)) return false
        config.options.documentPath = vf.path
        config.name = vf.nameWithoutExtension.ifBlank { vf.name }
        if (config.options.workingDirectory.isNullOrBlank()) {
            // Documents read their neighbours, so the directory the file is in
            // is the only working directory that makes a run reproducible.
            config.options.workingDirectory = vf.parent?.path ?: ""
        }
        return true
    }

    override fun isConfigurationFromContext(
        config: TexrsRunConfiguration,
        context: ConfigurationContext,
    ): Boolean {
        val vf = context.psiLocation?.containingFile?.virtualFile ?: return false
        return TexrsSettings.getInstance().isSupportedFile(vf.name, vf.extension) &&
            config.options.documentPath == vf.path
    }
}
