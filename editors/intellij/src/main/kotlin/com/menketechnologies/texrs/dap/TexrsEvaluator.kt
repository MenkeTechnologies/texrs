package com.menketechnologies.texrs.dap

import com.google.gson.JsonObject
import com.intellij.openapi.application.ApplicationManager
import com.intellij.xdebugger.XSourcePosition
import com.intellij.xdebugger.evaluation.XDebuggerEvaluator

/// Evaluate Expression, and the value shown when the mouse rests on a name.
class TexrsEvaluator(
    private val client: TexrsDapClient?,
    private val frameId: Int,
) : XDebuggerEvaluator() {

    override fun evaluate(
        expression: String,
        callback: XEvaluationCallback,
        expressionPosition: XSourcePosition?,
    ) {
        val c = client
        if (c == null || !c.isAlive()) {
            callback.errorOccurred("the debugger is not connected")
            return
        }
        ApplicationManager.getApplication().executeOnPooledThread {
            try {
                val args = JsonObject().apply {
                    addProperty("expression", expression)
                    addProperty("frameId", frameId)
                    addProperty("context", "watch")
                }
                val body = c.request("evaluate", args)
                if (body == null) {
                    callback.errorOccurred("the debugger did not answer")
                    return@executeOnPooledThread
                }
                callback.evaluated(
                    TexrsValue(
                        name = expression,
                        repr = body.get("result")?.asString ?: "",
                        kind = body.get("type")?.asString ?: "",
                        varRef = body.get("variablesReference")?.asInt ?: 0,
                        client = c,
                    ),
                )
            } catch (e: Exception) {
                callback.errorOccurred(e.message ?: "the expression could not be evaluated")
            }
        }
    }
}
