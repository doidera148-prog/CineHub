package com.cinehub.exoplayer

import org.junit.Assert.assertEquals
import org.junit.Test

class PlayerControllerFocusPolicyTest {
    @Test
    fun focusedTrackSelectorDisablesControllerAutoHide() {
        assertEquals(0, PlayerControllerFocusPolicy.timeoutMs(overlayControlHasFocus = true))
    }

    @Test
    fun leavingTrackSelectorsRestoresNormalAutoHide() {
        assertEquals(
            PlayerControllerFocusPolicy.AUTO_HIDE_TIMEOUT_MS,
            PlayerControllerFocusPolicy.timeoutMs(overlayControlHasFocus = false),
        )
        assertEquals(6_000, PlayerControllerFocusPolicy.AUTO_HIDE_TIMEOUT_MS)
    }
}
