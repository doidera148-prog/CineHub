package com.cinehub.exoplayer

import com.cinehub.exoplayer.PlayerControllerFocusPolicy.FocusDirection
import com.cinehub.exoplayer.PlayerControllerFocusPolicy.FocusNode
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
        assertEquals(
            FocusNode.AUDIO,
            PlayerControllerFocusPolicy.nextFocusTarget(FocusNode.PLAY_PAUSE, FocusDirection.UP, true),
        )
        assertEquals(
            FocusNode.SUBTITLES,
            PlayerControllerFocusPolicy.nextFocusTarget(FocusNode.PLAY_PAUSE, FocusDirection.UP, false),
        )
        assertEquals(
            FocusNode.PROGRESS,
            PlayerControllerFocusPolicy.nextFocusTarget(FocusNode.PLAY_PAUSE, FocusDirection.DOWN, true),
        )
        assertEquals(
            FocusNode.SUBTITLES,
            PlayerControllerFocusPolicy.nextFocusTarget(FocusNode.AUDIO, FocusDirection.DOWN, true),
        )
        assertEquals(
            FocusNode.PROGRESS,
            PlayerControllerFocusPolicy.nextFocusTarget(FocusNode.SUBTITLES, FocusDirection.DOWN, true),
        )
        assertEquals(
            FocusNode.SUBTITLES,
            PlayerControllerFocusPolicy.nextFocusTarget(FocusNode.PROGRESS, FocusDirection.UP, true),
        )
        assertEquals(
            FocusNode.PLAY_PAUSE,
            PlayerControllerFocusPolicy.nextFocusTarget(FocusNode.PROGRESS, FocusDirection.DOWN, true),
        )
    }

    @Test
    fun dpadCanReachSkipButtonOnlyWhileASegmentIsActive() {
        assertEquals(
            FocusNode.SKIP,
            PlayerControllerFocusPolicy.nextFocusTarget(
                FocusNode.AUDIO, FocusDirection.LEFT, true, hasSkipButton = true,
            ),
        )
        assertEquals(
            FocusNode.SKIP,
            PlayerControllerFocusPolicy.nextFocusTarget(
                FocusNode.SUBTITLES, FocusDirection.LEFT, false, hasSkipButton = true,
            ),
        )
        assertEquals(
            FocusNode.PLAY_PAUSE,
            PlayerControllerFocusPolicy.nextFocusTarget(
                FocusNode.AUDIO, FocusDirection.LEFT, true, hasSkipButton = false,
            ),
        )
        assertEquals(
            FocusNode.PLAY_PAUSE,
            PlayerControllerFocusPolicy.nextFocusTarget(
                FocusNode.SUBTITLES, FocusDirection.LEFT, false, hasSkipButton = false,
            ),
        )
        assertEquals(
            FocusNode.AUDIO,
            PlayerControllerFocusPolicy.nextFocusTarget(
                FocusNode.SKIP, FocusDirection.RIGHT, true, hasSkipButton = true,
            ),
        )
        assertEquals(
            FocusNode.SUBTITLES,
            PlayerControllerFocusPolicy.nextFocusTarget(
                FocusNode.SKIP, FocusDirection.RIGHT, false, hasSkipButton = true,
            ),
        )
        assertEquals(
            FocusNode.PLAY_PAUSE,
            PlayerControllerFocusPolicy.nextFocusTarget(
                FocusNode.SKIP, FocusDirection.DOWN, false, hasSkipButton = true,
            ),
        )
    }

    @Test
    fun dpadReturnsFromTrackSelectorsAndSkipsUnavailableAudioSelector() {
        assertEquals(
            FocusNode.PLAY_PAUSE,
            PlayerControllerFocusPolicy.nextFocusTarget(FocusNode.SUBTITLES, FocusDirection.UP, false),
        )
        assertEquals(
            FocusNode.AUDIO,
            PlayerControllerFocusPolicy.nextFocusTarget(FocusNode.SUBTITLES, FocusDirection.UP, true),
        )
        assertEquals(
            FocusNode.PLAY_PAUSE,
            PlayerControllerFocusPolicy.nextFocusTarget(FocusNode.AUDIO, FocusDirection.LEFT, true),
        )
        assertEquals(
            FocusNode.PLAY_PAUSE,
            PlayerControllerFocusPolicy.nextFocusTarget(FocusNode.SUBTITLES, FocusDirection.LEFT, true),
        )
    }
}
