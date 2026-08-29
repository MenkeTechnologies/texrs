package com.menketechnologies.texrs

import com.intellij.openapi.fileChooser.FileChooserDescriptorFactory
import com.intellij.openapi.options.Configurable
import com.intellij.openapi.ui.TextFieldWithBrowseButton
import com.intellij.ui.components.JBCheckBox
import com.intellij.ui.components.JBLabel
import com.intellij.ui.components.JBTextField
import com.intellij.util.ui.FormBuilder
import com.intellij.util.ui.JBUI
import javax.swing.JComponent
import javax.swing.JPanel

/// Settings -> Tools -> texrs.
class TexrsSettingsConfigurable : Configurable {

    private val executableField = TextFieldWithBrowseButton().apply {
        addBrowseFolderListener(
            "texrs Executable",
            "Path to the texrs binary",
            null,
            FileChooserDescriptorFactory.createSingleFileNoJarsDescriptor(),
        )
    }
    private val fileExtensionsField = JBTextField()
    private val noCacheBox = JBCheckBox("Pass --no-cache on every run started from the IDE")

    private var panel: JPanel? = null

    override fun getDisplayName(): String = "texrs"

    override fun createComponent(): JComponent {
        val p = FormBuilder.createFormBuilder()
            .addComponent(sectionHeader("Engine"))
            .addLabeledComponent(JBLabel("texrs executable:"), executableField, 1, false)
            .addTooltip("Leave blank to use the first `texrs` on \$PATH.")

            .addComponent(sectionHeader("Editor"))
            .addLabeledComponent(JBLabel("File extensions:"), fileExtensionsField, 1, false)
            .addTooltip("Comma-separated, no leading dot. Default: `tex`.")

            .addComponent(sectionHeader("Cache"))
            .addComponent(noCacheBox)
            .addTooltip(
                "texrs caches compiled bytecode in ~/.cache/texrs/scripts.rkyv and reuses it " +
                    "when a document has not changed. Tick this to compile every time.",
            )

            .addComponentFillVertically(JPanel(), 0)
            .panel
        p.border = JBUI.Borders.empty(10)
        panel = p
        reset()
        return p
    }

    private fun sectionHeader(title: String) =
        JBLabel("<html><b>$title</b></html>").apply { border = JBUI.Borders.emptyTop(8) }

    override fun isModified(): Boolean {
        val s = TexrsSettings.getInstance()
        return executableField.text != (s.texrsExecutable ?: "") ||
            fileExtensionsField.text != s.fileExtensions ||
            noCacheBox.isSelected != s.passNoCache
    }

    override fun apply() {
        val s = TexrsSettings.getInstance()
        s.texrsExecutable = executableField.text.takeIf { it.isNotBlank() }
        s.fileExtensions = fileExtensionsField.text.ifBlank { "tex" }
        s.passNoCache = noCacheBox.isSelected
    }

    override fun reset() {
        val s = TexrsSettings.getInstance()
        executableField.text = s.texrsExecutable ?: ""
        fileExtensionsField.text = s.fileExtensions
        noCacheBox.isSelected = s.passNoCache
    }

    override fun disposeUIResources() {
        panel = null
    }
}
