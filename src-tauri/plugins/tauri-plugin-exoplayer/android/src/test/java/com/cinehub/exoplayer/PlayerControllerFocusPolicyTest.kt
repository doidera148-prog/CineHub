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

    @Test
    fun dpadConnectsPlaybackProgressAndAvailableTrackSelectors() {
        val policy = PlayerControllerFocusPolicy
        val node = PlayerControllerFocusPolicy.FocusNode
        val direction = PlayerControllerFocusPolicy.FocusDirection

        assertEquals(node.AUDIO, policy.nextFocusTarget(node.PLAY_PAUSE, direction.UP, true))
        assertEquals(node.SUBTITLES, policy.nextFocusTarget(node.PLAY_PAUSE, direction.UP, false))
        assertEquals(node.PROGRESS, policy.nextFocusTarget(node.PLAY_PAUSE, direction.DOWN, true))
        assertEquals(node.SUBTITLES, policy.nextFocusTarget(node.AUDIO, direction.DOWN, true))
        assertEquals(node.PROGRESS, policy.nextFocusTarget(node.SUBTITLES, direction.DOWN, true))
        assertEquals(node.SUBTITLES, policy.nextFocusTarget(node.PROGRESS, direction.UP, true))
        assertEquals(node.PLAY_PAUSE, policy.nextFocusTarget(node.PROGRESS, direction.DOWN, true))
    }

    @Test
    fun dpadReturnsFromTrackSelectorsAndSkipsUnavailableAudioSelector() {
        val policy = PlayerControllerFocusPolicy
        val node = PlayerControllerFocusPolicy.FocusNode
        val direction = PlayerControllerFocusPolicy.FocusDirection

        assertEquals(node.PLAY_PAUSE, policy.nextFocusTarget(node.SUBTITLES, direction.UP, false))
        assertEquals(node.AUDIO, policy.nextFocusTarget(node.SUBTITLES, direction.UP, true))
        assertEquals(node.PLAY_PAUSE, policy.nextFocusTarget(node.AUDIO, direction.LEFT, true))
        assertEquals(node.PLAY_PAUSE, policy.nextFocusTarget(node.SUBTITLES, direction.LEFT, true))
    }
}
