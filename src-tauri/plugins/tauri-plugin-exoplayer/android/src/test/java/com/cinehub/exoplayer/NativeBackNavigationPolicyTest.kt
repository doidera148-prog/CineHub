package com.cinehub.exoplayer

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class NativeBackNavigationPolicyTest {
    @Test
    fun webViewConsumesBackOnlyWhenItReturnsOne() {
        assertTrue(NativeBackNavigationPolicy.isHandledByWebView("1"))
    }

    @Test
    fun androidReceivesBackWhenWebViewDoesNotConsumeIt() {
        listOf(null, "0", "", "true", "null", " 1 ").forEach { result ->
            assertFalse(
                "Resultado inesperado do WebView: $result",
                NativeBackNavigationPolicy.isHandledByWebView(result),
            )
        }
    }
}
