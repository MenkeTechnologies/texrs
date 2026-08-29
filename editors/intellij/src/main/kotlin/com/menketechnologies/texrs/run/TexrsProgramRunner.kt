package com.menketechnologies.texrs.run

import com.intellij.execution.configurations.RunProfile
import com.intellij.execution.executors.DefaultRunExecutor
import com.intellij.execution.runners.DefaultProgramRunner

/**
 * Run only. There is no debug runner here because texrs ships no debug adapter
 * — the sibling plugins' Debug button is wired to `--dap`, which this engine
 * does not have, and a button that cannot work is worse than none.
 */
class TexrsProgramRunner : DefaultProgramRunner() {
    override fun getRunnerId(): String = "TexrsProgramRunner"

    override fun canRun(executorId: String, profile: RunProfile): Boolean {
        if (profile !is TexrsRunConfiguration) return false
        return executorId == DefaultRunExecutor.EXECUTOR_ID
    }
}
