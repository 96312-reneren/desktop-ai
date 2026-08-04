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
import java.util.concurrent.Executors

class MainActivity : Activity() {

    private val apiPool = Executors.newSingleThreadExecutor()

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        // 1. Start the Rust backend (model load + API server on 127.0.0.1).
        val internalDir = filesDir.absolutePath
        val externalDir = getExternalFilesDir(null)?.absolutePath ?: ""
        android.util.Log.i("DesktopAI", "onCreate: internal=$internalDir external=$externalDir")
        try {
            System.loadLibrary("desktop_ai")
            android.util.Log.i("DesktopAI", "loadLibrary ok, calling startRust")
            startRust(internalDir, externalDir, 11434)
            android.util.Log.i("DesktopAI", "startRust returned")
        } catch (e: Throwable) {
            android.util.Log.e("DesktopAI", "failed to start rust service", e)
        }

        // 2. WebView chat UI.
        val web = WebView(this)
        web.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            allowFileAccess = true
            setSupportMultipleWindows(false)
            cacheMode = WebSettings.LOAD_NO_CACHE
        }
        web.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(
                view: WebView?, request: WebResourceRequest?
            ): Boolean = false
        }
        web.webChromeClient = WebChromeClient()

        // 3. JS bridge: POSTs go through the Java network stack, which has no
        // CORS / Private-Network-Access restrictions.
        web.addJavascriptInterface(object {
            @JavascriptInterface
            fun postJson(url: String, body: String, cb: String) {
                apiPool.execute {
                    var result: String
                    try {
                        val conn = URL(url).openConnection() as HttpURLConnection
                        conn.requestMethod = "POST"
                        conn.setRequestProperty("Content-Type", "application/json")
                        conn.setRequestProperty("Authorization", "Bearer desktopai")
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

    private external fun startRust(internalDir: String, externalDir: String, apiPort: Int)
}
