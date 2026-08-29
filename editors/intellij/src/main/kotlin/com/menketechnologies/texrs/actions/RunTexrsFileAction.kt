package com.menketechnologies.texrs.actions

import com.intellij.execution.RunManager
import com.intellij.execution.executors.DefaultRunExecutor
import com.intellij.execution.runners.ExecutionUtil
import com.intellij.openapi.actionSystem.ActionUpdateThread
import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.actionSystem.CommonDataKeys
import com.menketechnologies.texrs.TexrsSettings
import com.menketechnologies.texrs.run.TexrsRunConfiguration
import com.menketechnologies.texrs.run.TexrsRunConfigurationType

class RunTexrsFileAction : AnAction() {
    override fun getActionUpdateThread(): ActionUpdateThread = ActionUpdateThread.BGT

    override fun update(e: AnActionEvent) {
        val vf = e.getData(CommonDataKeys.VIRTUAL_FILE)
        e.presentation.isEnabledAndVisible =
            vf != null && TexrsSettings.getInstance().isSupportedFile(vf.name, vf.extension)
    }

    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        val vf = e.getData(CommonDataKeys.VIRTUAL_FILE) ?: return
        val runManager = RunManager.getInstance(project)
        val factory = TexrsRunConfigurationType.getInstance().factory
        val name = "Run ${vf.nameWithoutExtension.ifBlank { vf.name }}"
        // Reuse the configuration this action made last time rather than
        // adding a second one with the same name on every invocation.
        val settings = runManager.findConfigurationByTypeAndName(factory.type.id, name)
            ?: runManager.createConfiguration(name, factory).also {
                val cfg = it.configuration as TexrsRunConfiguration
                cfg.options.documentPath = vf.path
                cfg.options.workingDirectory = vf.parent?.path ?: ""
                runManager.addConfiguration(it)
            }
        runManager.selectedConfiguration = settings
        ExecutionUtil.runConfiguration(settings, DefaultRunExecutor.getRunExecutorInstance())
    }
}
