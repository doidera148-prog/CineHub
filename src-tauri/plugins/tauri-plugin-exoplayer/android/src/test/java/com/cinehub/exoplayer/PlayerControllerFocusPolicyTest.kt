package com.cinehub.exoplayer

import com.cinehub.exoplayer.PlayerControllerFocusPolicy.FocusDirection
import com.cinehub.exoplayer.PlayerControllerFocusPolicy.FocusNode
import org.junit.Assert.assertEquals
import org.junit.Test

class PlayerControllerFocusPolicyTest {
    @Test
    fun nextEpisodeCountdownIsOfferedOnlyWhenANextEpisodeExists() {
        assertEquals(NextEpisodeAutoPlayPolicy.COUNTDOWN_SECONDS, 10)
        assertEquals(false, NextEpisodeAutoPlayPolicy.hasNextEpisode(null))
        assertEquals(false, NextEpisodeAutoPlayPolicy.hasNextEpisode("  "))
        assertEquals(true, NextEpisodeAutoPlayPolicy.hasNextEpisode("Série · T1E2"))
    }

    @Test
    fun nextEpisodeCountdownStartsOnlyAfterExpiryAndNeverAfterCancellation() {
        val hasNext = true
        assertEquals(false, NextEpisodeAutoPlayPolicy.shouldStart(hasNext, cancelled = false, secondsRemaining = 1))
        assertEquals(true, NextEpisodeAutoPlayPolicy.shouldStart(hasNext, cancelled = false, secondsRemaining = 0))
        assertEquals(true, NextEpisodeAutoPlayPolicy.shouldStart(hasNext, cancelled = false, secondsRemaining = -1))
        assertEquals(false, NextEpisodeAutoPlayPolicy.shouldStart(hasNext, cancelled = true, secondsRemaining = 0))
        assertEquals(false, NextEpisodeAutoPlayPolicy.shouldStart(false, cancelled = false, secondsRemaining = 0))
    }

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
    fun dpadCanReachNextEpisodeActionsAndReturnToPlayback() {
        assertEquals(
            FocusNode.NEXT_EPISODE,
            PlayerControllerFocusPolicy.nextFocusTarget(
                FocusNode.SUBTITLES, FocusDirection.LEFT, false, hasNextEpisode = true,
            ),
        )
        assertEquals(
            FocusNode.NEXT_CANCEL,
            PlayerControllerFocusPolicy.nextFocusTarget(
                FocusNode.NEXT_EPISODE, FocusDirection.RIGHT, false, hasNextEpisode = true,
            ),
        )
        assertEquals(
            FocusNode.NEXT_EPISODE,
            PlayerControllerFocusPolicy.nextFocusTarget(
                FocusNode.NEXT_CANCEL, FocusDirection.LEFT, false, hasNextEpisode = true,
            ),
        )
        assertEquals(
            FocusNode.SUBTITLES,
            PlayerControllerFocusPolicy.nextFocusTarget(
                FocusNode.NEXT_CANCEL, FocusDirection.UP, false, hasNextEpisode = true,
            ),
        )
        assertEquals(
            FocusNode.PLAY_PAUSE,
            PlayerControllerFocusPolicy.nextFocusTarget(
                FocusNode.NEXT_CANCEL, FocusDirection.DOWN, false, hasNextEpisode = true,
            ),
        )
        assertEquals(
            FocusNode.PLAY_PAUSE,
            PlayerControllerFocusPolicy.nextFocusTarget(
                FocusNode.NEXT_EPISODE, FocusDirection.DOWN, false, hasNextEpisode = true,
            ),
        )
        assertEquals(
            FocusNode.PLAY_PAUSE,
            PlayerControllerFocusPolicy.nextFocusTarget(
                FocusNode.SUBTITLES, FocusDirection.LEFT, false, hasNextEpisode = false,
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
