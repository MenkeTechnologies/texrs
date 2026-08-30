package com.menketechnologies.texrs.dap

import com.google.gson.JsonObject
import com.intellij.icons.AllIcons
import com.intellij.openapi.application.ApplicationManager
import com.intellij.xdebugger.frame.XCompositeNode
import com.intellij.xdebugger.frame.XNamedValue
import com.intellij.xdebugger.frame.XValueChildrenList
import com.intellij.xdebugger.frame.XValueNode
import com.intellij.xdebugger.frame.XValuePlace

/**
 * One row in the Variables window — a count register, a macro's meaning.
 *
 * A value with a non-zero `variablesReference` has children the adapter will
 * hand over on request, which is what puts an expand triangle beside it.
 */
class TexrsValue(
    name: String,
    private val repr: String,
    private val kind: String,
    private val varRef: Int = 0,
    private val client: TexrsDapClient? = null,
) : XNamedValue(name) {

    override fun computePresentation(node: XValueNode, place: XValuePlace) {
        val icon = when (kind) {
            "macro" -> AllIcons.Nodes.Function
            "register", "count" -> AllIcons.Debugger.Db_primitive
            else -> AllIcons.Debugger.Value
        }
        node.setPresentation(icon, kind, repr, varRef != 0)
    }

    override fun computeChildren(node: XCompositeNode) {
        val c = client
        if (varRef == 0 || c == null) {
            node.addChildren(XValueChildrenList.EMPTY, true)
            return
        }
        // Off the UI thread: the adapter is a process, and asking it anything
        // on the dispatch thread freezes the IDE for as long as it takes.
        ApplicationManager.getApplication().executeOnPooledThread {
            val args = JsonObject().apply { addProperty("variablesReference", varRef) }
            val body = c.request("variables", args)
            val list = XValueChildrenList()
            body?.getAsJsonArray("variables")?.forEach { entry ->
                val v = entry.asJsonObject
                list.add(
                    TexrsValue(
                        name = v.get("name")?.asString ?: "?",
                        repr = v.get("value")?.asString ?: "",
                        kind = v.get("type")?.asString ?: "",
                        varRef = v.get("variablesReference")?.asInt ?: 0,
                        client = c,
                    ),
                )
            }
            node.addChildren(list, true)
        }
    }

    override fun canNavigateToSource(): Boolean = false
}
