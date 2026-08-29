package com.menketechnologies.texrs.run

import com.intellij.openapi.fileChooser.FileChooserDescriptorFactory
import com.intellij.openapi.options.SettingsEditor
import com.intellij.openapi.ui.TextFieldWithBrowseButton
import com.intellij.ui.components.JBCheckBox
import com.intellij.ui.components.JBLabel
import com.intellij.ui.components.JBTextField
import com.intellij.util.ui.FormBuilder
import com.intellij.util.ui.JBUI
import javax.swing.JComponent
import javax.swing.JPanel

class TexrsRunConfigurationEditor : SettingsEditor<TexrsRunConfiguration>() {
    private val documentField = TextFieldWithBrowseButton().apply {
        addBrowseFolderListener(
            "TeX Document",
            "Choose a TeX document to run",
            null,
            FileChooserDescriptorFactory.createSingleFileNoJarsDescriptor(),
        )
    }
    private val engineArgsField = JBTextField()
    private val workDirField = TextFieldWithBrowseButton().apply {
        addBrowseFolderListener(
            "Working Directory",
            "Choose the run working directory",
            null,
            FileChooserDescriptorFactory.createSingleFolderDescriptor(),
        )
    }
    private val disasmCheck = JBCheckBox("--disasm (lowered fusevm bytecode)")
    private val dumpTokensCheck = JBCheckBox("--dump-tokens (the mouth's token stream)")
    private val noCacheCheck = JBCheckBox("--no-cache (compile rather than read the cache)")

    private val panel: JPanel = FormBuilder.createFormBuilder()
        .addComponent(header("Document"))
        .addLabeledComponent("Document:", documentField)
        .addLabeledComponent("Engine arguments:", engineArgsField)
        .addLabeledComponent("Working directory:", workDirField)

        .addComponent(header("Stop early"))
        .addComponent(dumpTokensCheck)
        .addComponent(disasmCheck)

        .addComponent(header("Cache"))
        .addComponent(noCacheCheck)

        .addComponentFillVertically(JPanel(), 0)
        .panel.apply { border = JBUI.Borders.empty(8) }

    private fun header(title: String) =
        JBLabel("<html><b>$title</b></html>").apply { border = JBUI.Borders.emptyTop(8) }

    override fun createEditor(): JComponent = panel

    override fun resetEditorFrom(s: TexrsRunConfiguration) {
        documentField.text = s.options.documentPath.orEmpty()
        engineArgsField.text = s.options.engineArgs.orEmpty()
        workDirField.text = s.options.workingDirectory.orEmpty()
        disasmCheck.isSelected = s.options.disasm
        dumpTokensCheck.isSelected = s.options.dumpTokens
        noCacheCheck.isSelected = s.options.noCache
    }

    override fun applyEditorTo(s: TexrsRunConfiguration) {
        s.options.documentPath = documentField.text
        s.options.engineArgs = engineArgsField.text
        s.options.workingDirectory = workDirField.text
        s.options.disasm = disasmCheck.isSelected
        s.options.dumpTokens = dumpTokensCheck.isSelected
        s.options.noCache = noCacheCheck.isSelected
    }
}
