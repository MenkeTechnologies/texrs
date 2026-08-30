package com.menketechnologies.texrs.dap

import com.google.gson.JsonObject
import com.google.gson.JsonParser
import com.intellij.openapi.diagnostic.Logger
import java.io.BufferedInputStream
import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.io.OutputStream
import java.nio.charset.StandardCharsets
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference

/**
 * The Debug Adapter Protocol over `texrs --dap`'s stdio, framed the way the
 * protocol frames it: `Content-Length` then a blank line then the body.
 *
 * The framing is done in BYTES rather than characters, which matters as soon as
 * a variable's value holds anything outside ASCII: a length counted in
 * characters desynchronises the stream, and every message after it is read at
 * the wrong offset.
 */
class TexrsDapClient(
    private val output: OutputStream,
    input: InputStream,
    private val onEvent: (event: String, body: JsonObject) -> Unit,
) {
    private val input = BufferedInputStream(input)
    private val seq = AtomicInteger(1)
    private val pending = ConcurrentHashMap<Int, AtomicReference<JsonObject?>>()
    private val pendingLatch = ConcurrentHashMap<Int, CountDownLatch>()

    @Volatile
    private var alive = true

    init {
        Thread({
            try {
                runReader()
            } catch (e: Exception) {
                LOG.warn("the DAP reader stopped", e)
            }
            alive = false
            // Whatever was waiting will never be answered now; waking it with
            // nothing is what turns a dead adapter into an error rather than a
            // hang.
            pendingLatch.values.forEach { it.countDown() }
        }, "texrs-dap-reader").apply {
            isDaemon = true
            start()
        }
    }

    fun isAlive(): Boolean = alive

    /// Send a request and wait for its response, or `null` if none arrives.
    fun request(
        command: String,
        arguments: JsonObject = JsonObject(),
        timeoutMs: Long = 10_000,
    ): JsonObject? {
        val s = seq.getAndIncrement()
        val latch = CountDownLatch(1)
        val slot = AtomicReference<JsonObject?>()
        pendingLatch[s] = latch
        pending[s] = slot
        send(requestMessage(s, command, arguments))
        latch.await(timeoutMs, TimeUnit.MILLISECONDS)
        pending.remove(s)
        pendingLatch.remove(s)
        return slot.get()
    }

    /// Send a request and do not wait — for the ones whose answer says nothing
    /// the caller acts on, such as re-sending a file's breakpoints.
    fun requestAsync(command: String, arguments: JsonObject = JsonObject()) {
        send(requestMessage(seq.getAndIncrement(), command, arguments))
    }

    private fun requestMessage(s: Int, command: String, arguments: JsonObject): JsonObject =
        JsonObject().apply {
            addProperty("seq", s)
            addProperty("type", "request")
            addProperty("command", command)
            add("arguments", arguments)
        }

    @Synchronized
    private fun send(msg: JsonObject) {
        if (!alive) return
        val body = msg.toString().toByteArray(StandardCharsets.UTF_8)
        val header = "Content-Length: ${body.size}\r\n\r\n".toByteArray(StandardCharsets.US_ASCII)
        try {
            output.write(header)
            output.write(body)
            output.flush()
        } catch (e: Exception) {
            LOG.warn("could not send to the debug adapter", e)
            alive = false
        }
    }

    private fun runReader() {
        while (alive) {
            val contentLength = readHeaders() ?: return
            if (contentLength <= 0) continue
            val bodyBytes = ByteArray(contentLength)
            var at = 0
            while (at < contentLength) {
                val n = input.read(bodyBytes, at, contentLength - at)
                if (n < 0) {
                    alive = false
                    return
                }
                at += n
            }
            val obj = try {
                JsonParser.parseString(String(bodyBytes, StandardCharsets.UTF_8)).asJsonObject
            } catch (_: Exception) {
                continue
            }
            dispatch(obj)
        }
    }

    /// Read up to the blank line, returning the Content-Length, or `null` when
    /// the adapter's stream ends.
    private fun readHeaders(): Int? {
        val header = ByteArrayOutputStream()
        while (true) {
            val b = input.read()
            if (b < 0) {
                alive = false
                return null
            }
            header.write(b)
            val bytes = header.toByteArray()
            val n = bytes.size
            if (n >= 4 &&
                bytes[n - 4] == 0x0d.toByte() && bytes[n - 3] == 0x0a.toByte() &&
                bytes[n - 2] == 0x0d.toByte() && bytes[n - 1] == 0x0a.toByte()
            ) {
                break
            }
        }
        var length = -1
        val text = String(header.toByteArray(), StandardCharsets.US_ASCII)
        for (line in text.split("\r\n")) {
            val at = line.indexOf(':')
            if (at <= 0) continue
            if (line.substring(0, at).trim().equals("Content-Length", ignoreCase = true)) {
                length = line.substring(at + 1).trim().toIntOrNull() ?: -1
            }
        }
        return length
    }

    private fun dispatch(obj: JsonObject) {
        when (obj.get("type")?.asString) {
            "response" -> {
                val requestSeq = obj.get("request_seq")?.asInt ?: return
                pending[requestSeq]?.set(obj.getAsJsonObject("body") ?: JsonObject())
                pendingLatch[requestSeq]?.countDown()
            }
            "event" -> {
                val event = obj.get("event")?.asString ?: return
                val body = obj.getAsJsonObject("body") ?: JsonObject()
                try {
                    onEvent(event, body)
                } catch (e: Exception) {
                    LOG.warn("the handler for event $event threw", e)
                }
            }
        }
    }

    fun close() {
        alive = false
        try {
            output.close()
        } catch (_: Exception) {
        }
    }

    companion object {
        private val LOG = Logger.getInstance(TexrsDapClient::class.java)
    }
}
