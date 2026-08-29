package com.menketechnologies.texrs.run

import com.intellij.execution.configurations.LocatableRunConfigurationOptions

class TexrsRunConfigurationOptions : LocatableRunConfigurationOptions() {
    var documentPath: String? by string()
    var engineArgs: String? by string()
    var workingDirectory: String? by string()

    /// `--disasm`: print the lowered fusevm bytecode instead of running it.
    var disasm: Boolean by property(false)

    /// `--dump-tokens`: stop after the mouth and print the token stream.
    var dumpTokens: Boolean by property(false)

    /// `--no-cache`: compile the document rather than reading the cache.
    var noCache: Boolean by property(false)
}
