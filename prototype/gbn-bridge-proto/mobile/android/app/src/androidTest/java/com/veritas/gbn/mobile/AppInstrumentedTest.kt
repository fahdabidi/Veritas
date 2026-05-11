package com.veritas.gbn.mobile

import androidx.test.platform.app.InstrumentationRegistry
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
}
