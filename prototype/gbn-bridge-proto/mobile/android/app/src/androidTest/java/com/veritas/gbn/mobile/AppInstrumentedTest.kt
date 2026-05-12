package com.veritas.gbn.mobile

import android.Manifest
import android.content.Intent
import android.os.Build
import android.os.SystemClock
import android.view.View
import android.view.ViewGroup
import android.widget.ScrollView
import android.widget.TextView
import androidx.test.platform.app.InstrumentationRegistry
import com.veritas.gbn.mobile.model.CreatorActionCatalog
import com.veritas.gbn.mobile.runtime.MobileCreatorRuntime
import com.veritas.gbn.mobile.runtime.RuntimeConfigFactory
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

class AppInstrumentedTest {
    @Test
    fun testNativeRuntimeLoadsAndReturnsMetadata() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val stateDir = File(context.filesDir, "instrumented-runtime")
        val evidenceDir = File(stateDir, "evidence")
        val config = RuntimeConfigFactory.build(
            stateDir = stateDir,
            evidenceDir = evidenceDir,
            creatorId = "phase3-instrumented",
            networkProfile = "offline_test",
            endpointConfigJson = null,
        )
        MobileCreatorRuntime(config).use { runtime ->
            val metadata = runtime.nodeMetadata()
            assertTrue(metadata.contains("phase3-instrumented"))
            assertTrue(metadata.contains("creator"))
        }
    }

    @Test
    fun testMainActivityClassIsAvailable() {
        assertNotNull(MainActivity::class.java)
    }

    @Test
    fun testMainActivityOfflineValidationWorkflow() {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        grantRuntimePermission(Manifest.permission.CAMERA)
        if (Build.VERSION.SDK_INT >= 33) {
            grantRuntimePermission(Manifest.permission.POST_NOTIFICATIONS)
        }

        val activity = instrumentation.startActivitySync(
            Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        ) as MainActivity
        instrumentation.waitForIdleSync()

        val root = activity.window.decorView
        val requiredIds = CreatorActionCatalog.requiredButtonIds + setOf(
            "StartRuntime",
            "StopRuntime",
            "ImportEndpointConfig",
            "PreviewBootstrapDHTQR",
            "ImportHostCreatorDHTSeed",
            "RefreshEvents",
            "MainScroll",
            "OperationOutput",
        )
        requiredIds.forEach { id ->
            assertNotNull("missing UI control $id", findByTag(root, id))
        }

        click(activity, "ShowNodeMetadata")
        waitForOutput(activity, "Runtime is stopped")
        waitForOutputScrolledIntoView(activity)
        click(activity, "StartRuntime")
        waitForOutput(activity, "creator")
        click(activity, "ShowNodeMetadata")
        waitForOutput(activity, "creator")
        waitForOutputScrolledIntoView(activity)
        click(activity, "PreviewBootstrapDHTQR")
        waitForOutput(activity, "host-creator")
        click(activity, "ImportHostCreatorDHTSeed")
        waitForOutput(activity, "host-creator")
        click(activity, "BuildUploadSession")
        waitForOutput(activity, "ciphertext_chunk_count")
        click(activity, "ExportEvidence")
        waitForOutput(activity, "Evidence ZIP")

        activity.finish()
    }

    private fun grantRuntimePermission(permission: String) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        runCatching {
            instrumentation.uiAutomation.grantRuntimePermission(
                instrumentation.targetContext.packageName,
                permission,
            )
        }
    }

    private fun click(activity: MainActivity, tag: String) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        instrumentation.runOnMainSync {
            val view = findByTag(activity.window.decorView, tag)
                ?: error("missing view $tag")
            view.performClick()
        }
        instrumentation.waitForIdleSync()
        SystemClock.sleep(500)
    }

    private fun waitForOutput(activity: MainActivity, expected: String) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        repeat(30) {
            var text = ""
            instrumentation.runOnMainSync {
                text = (findByTag(activity.window.decorView, "OperationOutput") as TextView).text.toString()
            }
            if (text.contains(expected)) return
            SystemClock.sleep(500)
        }
        instrumentation.runOnMainSync {
            val text = (findByTag(activity.window.decorView, "OperationOutput") as TextView).text.toString()
            error("expected output containing `$expected`, got `$text`")
        }
    }

    private fun waitForOutputScrolledIntoView(activity: MainActivity) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        repeat(30) {
            var scrollY = 0
            instrumentation.runOnMainSync {
                scrollY = (findByTag(activity.window.decorView, "MainScroll") as ScrollView).scrollY
            }
            if (scrollY > 0) return
            SystemClock.sleep(200)
        }
        instrumentation.runOnMainSync {
            val scrollY = (findByTag(activity.window.decorView, "MainScroll") as ScrollView).scrollY
            error("expected action output to scroll into view, got scrollY=$scrollY")
        }
    }

    private fun findByTag(root: View, tag: String): View? {
        if (root.tag == tag) return root
        if (root is ViewGroup) {
            for (index in 0 until root.childCount) {
                val found = findByTag(root.getChildAt(index), tag)
                if (found != null) return found
            }
        }
        return null
    }
}
