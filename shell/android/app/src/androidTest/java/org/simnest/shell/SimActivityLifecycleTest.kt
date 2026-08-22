package org.simnest.shell

import androidx.lifecycle.Lifecycle
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class SimActivityLifecycleTest {
    @Test
    fun recreationDenialSuspensionActivationAndCleanupUseTheNativeTable() {
        val scenario = ActivityScenario.launch(SimActivity::class.java)
        scenario.onActivity { activity ->
            assertFalse(activity.permissionResult("shared-document", false).getBoolean("accepted"))
            assertTrue(activity.testActivation("before-suspend").getBoolean("accepted"))
            assertEquals(0, activity.testLifecycle("suspended").getInt("resources"))
            assertFalse(activity.testActivation("while-suspended").getBoolean("accepted"))
            assertEquals(0, activity.testLifecycle("stopped").getInt("resources"))
        }
        scenario.recreate()
        scenario.moveToState(Lifecycle.State.RESUMED)
        scenario.onActivity { activity ->
            assertTrue(activity.testActivation("after-recreation").getBoolean("accepted"))
        }
        scenario.close()
    }
}
