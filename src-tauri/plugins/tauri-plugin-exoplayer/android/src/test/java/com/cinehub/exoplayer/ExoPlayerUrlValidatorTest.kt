package com.cinehub.exoplayer

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ExoPlayerUrlValidatorTest {
    @Test
    fun normalizesTrackLanguagesForPersistenceAcrossMediaFiles() {
        assertEquals("pt", TrackLanguagePreference.normalize("pt-BR"))
        assertEquals("pt", TrackLanguagePreference.normalize("por"))
        assertEquals("pt", TrackLanguagePreference.normalize("Português (Brasil)"))
        assertEquals("en", TrackLanguagePreference.normalize("eng"))
        assertEquals("en", TrackLanguagePreference.normalize("English"))
        assertEquals(null, TrackLanguagePreference.normalize("und"))
        assertEquals(null, TrackLanguagePreference.normalize(null))
    }

    @Test
    fun acceptsHttpStreamsIncludingLanAndLocalTorrentEndpoints() {
        assertTrue(ExoPlayerUrlValidator.isPlayableUrl("https://cdn.example/video.mkv"))
        assertTrue(ExoPlayerUrlValidator.isPlayableUrl("HTTP://cdn.example/video.mp4"))
        assertTrue(ExoPlayerUrlValidator.isPlayableUrl("http://127.0.0.1:43127/stream/token"))
        assertTrue(ExoPlayerUrlValidator.isPlayableUrl("http://192.168.1.20:8080/video.mp4"))
    }

    @Test
    fun rejectsExecutableLocalAndUnsupportedSchemes() {
        listOf(
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "file:///etc/passwd",
            "ftp://cdn.example/video.mkv",
            "/storage/emulated/0/video.mkv",
            "--script=/tmp/payload.lua",
            "",
        ).forEach { url ->
            assertFalse("URL não deveria ser reproduzível: $url", ExoPlayerUrlValidator.isPlayableUrl(url))
        }
    }

    @Test
    fun rejectsHttpUrlsWithoutAValidHost() {
        listOf("http://", "https:///video.mp4", "https://", "://missing-scheme").forEach { url ->
            assertFalse("URL sem host deveria ser recusada: $url", ExoPlayerUrlValidator.isPlayableUrl(url))
        }
    }

    @Test
    fun trimsWhitespaceBeforeValidation() {
        assertTrue(ExoPlayerUrlValidator.isPlayableUrl("  https://cdn.example/video.mkv  "))
    }
}
