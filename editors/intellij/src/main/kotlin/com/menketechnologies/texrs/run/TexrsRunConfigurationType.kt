package com.menketechnologies.texrs.run

import com.intellij.execution.configurations.ConfigurationFactory
import com.intellij.execution.configurations.ConfigurationType
import com.intellij.execution.configurations.RunConfiguration
import com.intellij.openapi.project.Project
import com.menketechnologies.texrs.TexrsIcons
import javax.swing.Icon

class TexrsRunConfigurationType : ConfigurationType {
    override fun getDisplayName(): String = "texrs"
    override fun getConfigurationTypeDescription(): String = "Run a TeX document with texrs"
    override fun getIcon(): Icon = TexrsIcons.FILE
    override fun getId(): String = "TEXRS_RUN_CONFIGURATION"
    override fun getConfigurationFactories(): Array<ConfigurationFactory> = arrayOf(factory)

    val factory = object : ConfigurationFactory(this) {
        override fun getId(): String = "texrs"
        override fun createTemplateConfiguration(project: Project): RunConfiguration =
            TexrsRunConfiguration(project, this, "texrs")
        override fun getOptionsClass(): Class<TexrsRunConfigurationOptions> =
            TexrsRunConfigurationOptions::class.java
    }

    companion object {
        fun getInstance(): TexrsRunConfigurationType =
            com.intellij.execution.configurations.ConfigurationTypeUtil
                .findConfigurationType(TexrsRunConfigurationType::class.java)
    }
}
