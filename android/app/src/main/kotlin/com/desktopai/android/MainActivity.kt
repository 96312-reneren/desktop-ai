package com.desktopai.android

import android.annotation.SuppressLint
import android.app.Activity
import android.os.Bundle
import android.webkit.JavascriptInterface
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL
import java.util.UUID
import java.util.concurrent.Executors

class MainActivity : Activity() {

    private val apiPool = Executors.newSingleThreadExecutor()
    private var apiPort = 0
    private var apiBase: String = ""

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        // 1. Start the Rust backend (model load + API server on 127.0.0.1).
        //    A fresh random token is generated in memory only — never stored,
        //    never embedded in the APK — and the port is allocated at runtime.
        val internalDir = filesDir.absolutePath
        val externalDir = getExternalFilesDir(null)?.absolutePath ?: ""
        val token = "dt-" + UUID.randomUUID().toString().replace("-", "")
        android.util.Log.i("DesktopAI", "onCreate: internal=$internalDir external=$externalDir")
        try {
            System.loadLibrary("desktop_ai")
            android.util.Log.i("DesktopAI", "loadLibrary ok, calling startRust")
            apiPort = startRust(internalDir, externalDir, token)
            android.util.Log.i("DesktopAI", "startRust returned port=$apiPort")
            if (apiPort <= 0) {
                android.util.Log.e("DesktopAI", "startRust failed (port=0)")
            }
        } catch (e: Throwable) {
            android.util.Log.e("DesktopAI", "failed to start rust service", e)
        }
        apiBase = "http://127.0.0.1:$apiPort"

        // 2. WebView chat UI. file access stays off: the page lives in
        //    android_asset and must not read other local files.
        val web = WebView(this)
        web.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            allowFileAccess = false
            setSupportMultipleWindows(false)
            cacheMode = WebSettings.LOAD_NO_CACHE
        }
        web.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(
                view: WebView?, request: WebResourceRequest?
            ): Boolean = false
        }
        web.webChromeClient = WebChromeClient()

        // 3. JS bridge: POSTs go through the Java network stack (no CORS /
        //    Private-Network-Access restrictions). The URL is whitelisted to
        //    the local API base so a compromised page cannot exfiltrate data.
        web.addJavascriptInterface(object {
            @JavascriptInterface
            fun postJson(url: String, body: String, cb: String) {
                val allowed = apiBase + "/v1/"
                if (!url.startsWith(allowed)) {
                    runOnUiThread {
                        web.evaluateJavascript(
                            "window.dispatchEvent(new CustomEvent('aiResp',{detail:" +
                                JSONObject.quote("{\"error\":\"url not allowed\"}") + "}));", null
                        )
                    }
                    return
                }
                apiPool.execute {
                    var result: String
                    try {
                        val conn = URL(url).openConnection() as HttpURLConnection
                        conn.requestMethod = "POST"
                        conn.setRequestProperty("Content-Type", "application/json")
                        conn.setRequestProperty("Authorization", "Bearer $token")
                        conn.setRequestProperty("Connection", "close")
                        conn.doOutput = true
                        conn.connectTimeout = 20000
                        conn.readTimeout = 300000
                        conn.outputStream.use { it.write(body.toByteArray(Charsets.UTF_8)) }
                        val resp = conn.inputStream.bufferedReader(Charsets.UTF_8).readText()
                        result = resp
                    } catch (e: Exception) {
                        result = "{\"error\":\"" + e.toString().replace("\"", "'") + "\"}"
                    }
                    runOnUiThread {
                        web.evaluateJavascript(
                            "window.dispatchEvent(new CustomEvent('aiResp',{detail:" +
                                JSONObject.quote(result) + "}));", null
                        )
                    }
                }
            }
        }, "AndroidBridge")

        setContentView(web)
        web.loadUrl("file:///android_asset/chat.html")
    }

    override fun onDestroy() {
        super.onDestroy()
        apiPool.shutdownNow()
    }

    private external fun startRust(internalDir: String, externalDir: String, apiToken: String): Int
}
