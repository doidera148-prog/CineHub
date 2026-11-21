package com.cinehub.exoplayer

import android.app.Activity
import android.content.Context
import android.app.AlertDialog
import android.app.Dialog
import android.graphics.Color
import android.view.ViewGroup
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import android.net.Uri
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MimeTypes
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.Tracks
import androidx.media3.common.TrackSelectionOverride
import androidx.media3.datasource.DefaultDataSource
import androidx.media3.datasource.DefaultHttpDataSource
import androidx.media3.exoplayer.DefaultRenderersFactory
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.ui.AspectRatioFrameLayout
import androidx.media3.ui.PlayerView
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.KeyEvent
import android.view.View
import android.widget.Button
import android.webkit.WebView
import android.util.Base64
import java.io.File
import org.json.JSONArray
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback

@InvokeArg
class PlayArgs {
    lateinit var url: String
    var startMs: Long = 0
    /** Lista JSON de legendas VTT em base64, com idioma, rótulo e preferência inicial. */
    var subtitles: String? = null
    var subtitlesDisabled: Boolean = false
    var audioLanguage: String? = null
    var subtitleLanguage: String? = null
    /** Rótulo do próximo episódio, quando houver, para oferecer reprodução automática ao terminar. */
    var nextEpisodeLabel: String? = null
    /** Trechos para pular, como JSON: [{"type":"intro","start":5,"end":90}, ...] (opcional). */
    var segments: String? = null
}

internal object TrackLanguagePreference {
    fun normalize(value: String?): String? {
        val raw = value?.trim()?.lowercase()?.replace('_', '-') ?: return null
        if (raw.isBlank() || raw == "und" || raw == "unknown") return null
        val token = raw.substringBefore('-').substringBefore(' ')
        return when (token) {
            "por", "pt", "portuguese", "português" -> "pt"
            "eng", "en", "english" -> "en"
            "spa", "es", "spanish" -> "es"
            "fre", "fra", "fr", "french" -> "fr"
            "ger", "deu", "de", "german" -> "de"
            "ita", "it" -> "it"
            "jpn", "ja" -> "ja"
            "zho", "chi", "zh" -> "zh"
            "kor", "ko" -> "ko"
            "rus", "ru" -> "ru"
            else -> token.takeIf { it.matches(Regex("[a-z]{2,3}")) }
        }
    }
}

/** Validação isolada para permitir testes unitários sem iniciar a Activity/player. */
internal object ExoPlayerUrlValidator {
    fun isPlayableUrl(raw: String): Boolean {
        return try {
            // java.net.URI funciona em testes JVM locais; android.net.Uri é um stub
            // nesses testes e retorna valores nulos fora de um dispositivo Android.
            val uri = java.net.URI(raw.trim())
            (uri.scheme.equals("http", ignoreCase = true) ||
                uri.scheme.equals("https", ignoreCase = true)) &&
                !uri.host.isNullOrBlank()
        } catch (_: Exception) {
            false
        }
    }
}

/** Mantém os seletores externos acessíveis enquanto recebem foco pelo D-pad. */
internal object PlayerControllerFocusPolicy {
    const val AUTO_HIDE_TIMEOUT_MS = 6_000

    enum class FocusNode { PLAY_PAUSE, PROGRESS, AUDIO, SUBTITLES, SKIP, NEXT_EPISODE, NEXT_CANCEL }
    enum class FocusDirection { UP, DOWN, LEFT, RIGHT }

    fun timeoutMs(overlayControlHasFocus: Boolean): Int =
        if (overlayControlHasFocus) 0 else AUTO_HIDE_TIMEOUT_MS

    /** Caminho previsível do D-pad entre reprodução, progresso e controles sobrepostos. */
    fun nextFocusTarget(
        current: FocusNode,
        direction: FocusDirection,
        hasAudioSelector: Boolean,
        hasSkipButton: Boolean = false,
        hasNextEpisode: Boolean = false,
    ): FocusNode? = when (current) {
        FocusNode.PLAY_PAUSE -> when (direction) {
            FocusDirection.UP -> if (hasAudioSelector) FocusNode.AUDIO else FocusNode.SUBTITLES
            FocusDirection.DOWN -> FocusNode.PROGRESS
            FocusDirection.LEFT, FocusDirection.RIGHT -> null
        }
        FocusNode.PROGRESS -> when (direction) {
            FocusDirection.UP -> FocusNode.SUBTITLES
            FocusDirection.DOWN -> FocusNode.PLAY_PAUSE
            FocusDirection.LEFT, FocusDirection.RIGHT -> null
        }
        FocusNode.AUDIO -> when (direction) {
            FocusDirection.UP -> null
            FocusDirection.DOWN -> FocusNode.SUBTITLES
            FocusDirection.LEFT -> when {
                hasNextEpisode -> FocusNode.NEXT_EPISODE
                hasSkipButton -> FocusNode.SKIP
                else -> FocusNode.PLAY_PAUSE
            }
            FocusDirection.RIGHT -> null
        }
        FocusNode.SUBTITLES -> when (direction) {
            FocusDirection.UP -> if (hasAudioSelector) FocusNode.AUDIO else FocusNode.PLAY_PAUSE
            FocusDirection.DOWN -> FocusNode.PROGRESS
            FocusDirection.LEFT -> when {
                hasNextEpisode -> FocusNode.NEXT_EPISODE
                hasSkipButton -> FocusNode.SKIP
                else -> FocusNode.PLAY_PAUSE
            }
            FocusDirection.RIGHT -> null
        }
        FocusNode.SKIP -> when (direction) {
            FocusDirection.UP, FocusDirection.LEFT -> null
            FocusDirection.DOWN -> FocusNode.PLAY_PAUSE
            FocusDirection.RIGHT -> when {
                hasNextEpisode -> FocusNode.NEXT_EPISODE
                hasAudioSelector -> FocusNode.AUDIO
                else -> FocusNode.SUBTITLES
            }
        }
        FocusNode.NEXT_EPISODE -> when (direction) {
            FocusDirection.UP -> FocusNode.SUBTITLES
            FocusDirection.DOWN -> FocusNode.PLAY_PAUSE
            FocusDirection.LEFT -> if (hasSkipButton) FocusNode.SKIP else null
            FocusDirection.RIGHT -> FocusNode.NEXT_CANCEL
        }
        FocusNode.NEXT_CANCEL -> when (direction) {
            FocusDirection.UP -> FocusNode.SUBTITLES
            FocusDirection.DOWN -> FocusNode.PLAY_PAUSE
            FocusDirection.LEFT -> FocusNode.NEXT_EPISODE
            FocusDirection.RIGHT -> if (hasAudioSelector) FocusNode.AUDIO else FocusNode.SUBTITLES
        }
    }
}

/** Regras puras do temporizador de reprodução automática do próximo episódio. */
internal object NextEpisodeAutoPlayPolicy {
    const val COUNTDOWN_SECONDS = 10

    fun hasNextEpisode(label: String?): Boolean = !label.isNullOrBlank()

    fun shouldStart(hasNextEpisode: Boolean, cancelled: Boolean, secondsRemaining: Int): Boolean =
        hasNextEpisode && !cancelled && secondsRemaining <= 0

    fun shouldDismissCountdownOnBack(panelVisible: Boolean, countdownActive: Boolean): Boolean =
        panelVisible && countdownActive

    fun shouldShowCompletionExit(playbackEnded: Boolean, hasNextEpisode: Boolean): Boolean =
        playbackEnded && !hasNextEpisode
}

/** Trecho a pular (abertura, recapitulação, créditos), em segundos. */
private data class Skip(val kind: String, val start: Double, val end: Double) {
    val label: String
        get() = when (kind) {
            "intro" -> "Pular abertura"
            "recap" -> "Pular recapitulação"
            "preview" -> "Pular prévia"
            "credits" -> "Pular créditos"
            else -> "Pular"
        }
}

/** Player Android/Google TV em tela cheia, com controles Media3 acessíveis por toque e D-pad. */
@TauriPlugin
class ExoPlayerPlugin(private val act: Activity) : Plugin(act) {
    /**
     * Botão Voltar: a página decide primeiro (fecha diálogo ou player, volta uma tela, ou pede
     * "Voltar de novo para sair", em window.__cinehubBack). Só quando ela não trata, o sistema segue.
     */
    override fun load(webView: WebView) {
        super.load(webView)
        val dispatcher = (act as? ComponentActivity)?.onBackPressedDispatcher ?: return
        val callback = object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                webView.evaluateJavascript("(window.__cinehubBack && window.__cinehubBack()) ? 1 : 0") { handled ->
                    if (handled == "1") return@evaluateJavascript
                    isEnabled = false
                    dispatcher.onBackPressed()
                    isEnabled = true
                }
            }
        }
        dispatcher.addCallback(callback)
    }

    @Command
    fun play(invoke: Invoke) {
        val args = try {
            invoke.parseArgs(PlayArgs::class.java)
        } catch (e: Exception) {
            invoke.reject("argumentos inválidos para o player nativo")
            return
        }
        if (!ExoPlayerUrlValidator.isPlayableUrl(args.url)) {
            invoke.reject("endereço de vídeo inválido")
            return
        }
        act.runOnUiThread { openPlayer(invoke, args) }
    }

    private fun openPlayer(invoke: Invoke, args: PlayArgs) {
        val renderers = DefaultRenderersFactory(act)
            .setEnableDecoderFallback(true)
            .setExtensionRendererMode(DefaultRenderersFactory.EXTENSION_RENDERER_MODE_ON)
        // Torrents podem levar vários segundos para entregar o próximo bloco quando há
        // poucos peers. O timeout HTTP padrão do ExoPlayer é curto para esse cenário,
        // causando "IO network timeout" mesmo com o servidor local ainda aguardando dados.
        val httpDataSource = DefaultHttpDataSource.Factory()
            .setConnectTimeoutMs(30_000)
            .setReadTimeoutMs(120_000)
            .setAllowCrossProtocolRedirects(true)
        val dataSource = DefaultDataSource.Factory(act, httpDataSource)
        val mediaSource = DefaultMediaSourceFactory(act).setDataSourceFactory(dataSource)
        val player = ExoPlayer.Builder(act, renderers)
            .setMediaSourceFactory(mediaSource)
            .build()
        val preferences = act.getSharedPreferences("cinehub_player_preferences", Context.MODE_PRIVATE)
        val preferredAudio = TrackLanguagePreference.normalize(preferences.getString("audioLanguage", null))
            ?: TrackLanguagePreference.normalize(args.audioLanguage)
        val preferredSubtitle = TrackLanguagePreference.normalize(preferences.getString("subtitleLanguage", null))
            ?: TrackLanguagePreference.normalize(args.subtitleLanguage)
        // A escolha feita pelo usuário prevalece; sem escolha salva, mantém português como padrão.
        val audioLanguages = listOfNotNull(preferredAudio, "pt-BR", "pt").distinct()
        val subtitleLanguages = listOfNotNull(preferredSubtitle, "pt-BR", "pt").distinct()
        player.trackSelectionParameters = player.trackSelectionParameters.buildUpon()
            .setPreferredAudioLanguages(*audioLanguages.toTypedArray())
            .setPreferredTextLanguages(*subtitleLanguages.toTypedArray())
            .setTrackTypeDisabled(C.TRACK_TYPE_TEXT, preferences.getBoolean("subtitlesDisabled", args.subtitlesDisabled))
            .build()

        val view = PlayerView(act).apply {
            this.player = player
            useController = true
            controllerShowTimeoutMs = PlayerControllerFocusPolicy.AUTO_HIDE_TIMEOUT_MS
            resizeMode = AspectRatioFrameLayout.RESIZE_MODE_FIT
            keepScreenOn = true
            isFocusable = true
            isFocusableInTouchMode = true
            descendantFocusability = ViewGroup.FOCUS_AFTER_DESCENDANTS
            setShowBuffering(PlayerView.SHOW_BUFFERING_WHEN_PLAYING)
        }
        val dialog = Dialog(act, android.R.style.Theme_Black_NoTitleBar_Fullscreen)

        // Guarda a preferência pelo idioma da faixa, pois o ID muda entre arquivos.
        var availableAudioTracks: List<Pair<Tracks.Group, Int>> = emptyList()
        lateinit var audioButton: Button
        audioButton = Button(act).apply {
            id = View.generateViewId()
            text = "Áudio"
            isAllCaps = false
            visibility = View.GONE
            isFocusable = true
            isFocusableInTouchMode = true
            setOnClickListener {
                val tracks = availableAudioTracks
                if (tracks.isEmpty()) return@setOnClickListener
                val choices = tracks.mapIndexed { index, entry ->
                    val format = entry.first.getTrackFormat(entry.second)
                    format.label?.takeIf { it.isNotBlank() }
                        ?: format.language?.takeIf { it.isNotBlank() }
                        ?: "Áudio ${index + 1}"
                }
                val selected = tracks.indexOfFirst { it.first.isTrackSelected(it.second) }
                AlertDialog.Builder(act).setTitle("Faixa de áudio")
                    .setSingleChoiceItems(choices.toTypedArray(), selected) { dialog, which ->
                        val (group, trackIndex) = tracks[which]
                        player.trackSelectionParameters = player.trackSelectionParameters.buildUpon()
                            .clearOverridesOfType(C.TRACK_TYPE_AUDIO)
                            .setTrackTypeDisabled(C.TRACK_TYPE_AUDIO, false)
                            .addOverride(TrackSelectionOverride(group.mediaTrackGroup, listOf(trackIndex)))
                            .build()
                        val format = group.getTrackFormat(trackIndex)
                        val language = TrackLanguagePreference.normalize(format.language)
                            ?: TrackLanguagePreference.normalize(format.label)
                        val editor = preferences.edit()
                        if (language == null) editor.remove("audioLanguage")
                        else editor.putString("audioLanguage", language)
                        editor.apply()
                        audioButton.text = "Áudio: ${choices[which]} ▾"
                        dialog.dismiss()
                    }.setNegativeButton("Fechar", null).show()
            }
        }
        // Seletor de legendas: lista todas as faixas e oferece a opção Desativadas.
        var availableSubtitleTracks: List<Pair<Tracks.Group, Int>> = emptyList()
        lateinit var subtitleButton: Button
        subtitleButton = Button(act).apply {
            id = View.generateViewId()
            text = "Legendas ▾"
            isAllCaps = false
            visibility = View.GONE
            isFocusable = true
            isFocusableInTouchMode = true
            setOnClickListener {
                val tracks = availableSubtitleTracks
                if (tracks.isEmpty()) {
                    AlertDialog.Builder(act).setTitle("Legendas")
                        .setMessage("Nenhuma legenda disponível nesta fonte.")
                        .setPositiveButton("OK", null).show()
                } else {
                    val choices = mutableListOf("Desativadas")
                    tracks.forEachIndexed { index, entry ->
                        val format = entry.first.getTrackFormat(entry.second)
                        val language = format.language?.takeIf { it.isNotBlank() }
                        val label = format.label?.takeIf { it.isNotBlank() }
                            ?: language?.let { "Legenda ($it)" } ?: ("Legenda " + (index + 1))
                        choices.add(label)
                    }
                    val selected = if (player.trackSelectionParameters.disabledTrackTypes.contains(C.TRACK_TYPE_TEXT)) 0
                        else tracks.indexOfFirst { it.first.isTrackSelected(it.second) }.let { if (it < 0) -1 else it + 1 }
                    AlertDialog.Builder(act).setTitle("Legendas")
                        .setSingleChoiceItems(choices.toTypedArray(), selected) { dialog, which ->
                            val builder = player.trackSelectionParameters.buildUpon()
                                .clearOverridesOfType(C.TRACK_TYPE_TEXT)
                            if (which == 0) {
                                builder.setTrackTypeDisabled(C.TRACK_TYPE_TEXT, true)
                                preferences.edit().putBoolean("subtitlesDisabled", true)
                                    .remove("subtitleLanguage").apply()
                                subtitleButton.text = "Legendas: desativadas ▾"
                            } else {
                                val (group, trackIndex) = tracks[which - 1]
                                builder.setTrackTypeDisabled(C.TRACK_TYPE_TEXT, false)
                                    .addOverride(TrackSelectionOverride(group.mediaTrackGroup, listOf(trackIndex)))
                                val format = group.getTrackFormat(trackIndex)
                                val language = TrackLanguagePreference.normalize(format.language)
                            ?: TrackLanguagePreference.normalize(format.label)
                                val editor = preferences.edit().putBoolean("subtitlesDisabled", false)
                                if (language == null) editor.remove("subtitleLanguage")
                                else editor.putString("subtitleLanguage", language)
                                editor.apply()
                                subtitleButton.text = "Legendas: " + choices[which] + " ▾"
                            }
                            player.trackSelectionParameters = builder.build()
                            dialog.dismiss()
                        }.setNegativeButton("Fechar", null).show()
                }
            }
        }
        // O botão de pular trecho é criado antes do mapa de foco para participar
        // da navegação D-pad assim que um segmento ficar ativo.
        val handler = Handler(Looper.getMainLooper())
        val density = act.resources.displayMetrics.density
        val skipButton = Button(act).apply {
            id = View.generateViewId()
            visibility = View.GONE
            isAllCaps = false
            isFocusable = true
            isFocusableInTouchMode = true
        }
        val nextEpisodeLabel = args.nextEpisodeLabel?.trim()
            ?.takeIf { NextEpisodeAutoPlayPolicy.hasNextEpisode(it) }
        var playbackEnded = false
        var finishedWithoutNextEpisode = false
        var nextEpisodeCountdownActive = false
        var nextEpisodeCountdownCancelled = false
        var autoNextRequested = false
        var countdownSecondsRemaining = NextEpisodeAutoPlayPolicy.COUNTDOWN_SECONDS
        val nextEpisodeButton = Button(act).apply {
            id = View.generateViewId()
            text = "Reproduzir agora"
            isAllCaps = false
            isFocusable = true
            isFocusableInTouchMode = true
        }
        val cancelNextEpisodeButton = Button(act).apply {
            id = View.generateViewId()
            text = "Cancelar"
            isAllCaps = false
            isFocusable = true
            isFocusableInTouchMode = true
        }
        val nextEpisodeStatus = TextView(act).apply {
            textSize = 18f
            setTextColor(Color.WHITE)
            gravity = Gravity.CENTER
            maxLines = 2
            isFocusable = false
            isFocusableInTouchMode = false
        }
        val nextEpisodeActions = LinearLayout(act).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER
            isFocusable = false
            addView(nextEpisodeButton)
            addView(
                cancelNextEpisodeButton,
                LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                ).apply { leftMargin = (12 * density).toInt() },
            )
        }
        val nextEpisodePanel = LinearLayout(act).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER
            visibility = View.GONE
            isFocusable = false
            addView(
                nextEpisodeStatus,
                LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                ).apply { bottomMargin = (8 * density).toInt() },
            )
            addView(
                nextEpisodeActions,
                LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                ),
            )
        }
        var hasMultipleAudioTracks = false

        // A barra de progresso do Media3 (DefaultTimeBar) já aceita LEFT/RIGHT para
        // avançar e retroceder; aqui conectamos o movimento vertical aos controles
        // externos para que o D-pad não dependa apenas da busca geométrica do Android.
        fun updatePlayerFocusNavigation() {
            val focusViews = mutableMapOf<PlayerControllerFocusPolicy.FocusNode, View?>(
                PlayerControllerFocusPolicy.FocusNode.PLAY_PAUSE to
                    view.findViewById<View>(androidx.media3.ui.R.id.exo_play_pause),
                PlayerControllerFocusPolicy.FocusNode.PROGRESS to
                    view.findViewById<View>(androidx.media3.ui.R.id.exo_progress),
                PlayerControllerFocusPolicy.FocusNode.AUDIO to audioButton,
                PlayerControllerFocusPolicy.FocusNode.SUBTITLES to subtitleButton,
            )
            val hasSkipButton = skipButton.visibility == View.VISIBLE
            val hasNextEpisode = nextEpisodePanel.visibility == View.VISIBLE
            if (hasSkipButton) focusViews[PlayerControllerFocusPolicy.FocusNode.SKIP] = skipButton
            if (hasNextEpisode) {
                focusViews[PlayerControllerFocusPolicy.FocusNode.NEXT_EPISODE] = nextEpisodeButton
                if (cancelNextEpisodeButton.visibility == View.VISIBLE) {
                    focusViews[PlayerControllerFocusPolicy.FocusNode.NEXT_CANCEL] = cancelNextEpisodeButton
                }
            }

            for ((node, source) in focusViews) {
                if (source == null) continue
                for (direction in PlayerControllerFocusPolicy.FocusDirection.values()) {
                    val targetNode = PlayerControllerFocusPolicy.nextFocusTarget(
                        node,
                        direction,
                        hasAudioSelector = hasMultipleAudioTracks && audioButton.visibility == View.VISIBLE,
                        hasSkipButton = hasSkipButton,
                        hasNextEpisode = hasNextEpisode,
                    ) ?: continue
                    val target = focusViews[targetNode] ?: continue
                    if (target.id == View.NO_ID) continue
                    when (direction) {
                        PlayerControllerFocusPolicy.FocusDirection.UP -> source.nextFocusUpId = target.id
                        PlayerControllerFocusPolicy.FocusDirection.DOWN -> source.nextFocusDownId = target.id
                        PlayerControllerFocusPolicy.FocusDirection.LEFT -> source.nextFocusLeftId = target.id
                        PlayerControllerFocusPolicy.FocusDirection.RIGHT -> source.nextFocusRightId = target.id
                    }
                }
            }
        }

        fun updateTrackButtons(visible: Boolean) {
            // Não remova da hierarquia um seletor que ainda está focado pelo D-pad.
            val keepFocusedOverlayButton = audioButton.hasFocus() || subtitleButton.hasFocus() ||
                skipButton.hasFocus() || nextEpisodeButton.hasFocus() || cancelNextEpisodeButton.hasFocus()
            val shouldShow = visible || keepFocusedOverlayButton
            audioButton.visibility = if (hasMultipleAudioTracks && shouldShow) View.VISIBLE else View.GONE
            // Botão fica disponível mesmo sem faixa para explicar que a fonte não oferece legenda.
            subtitleButton.visibility = if (shouldShow) View.VISIBLE else View.GONE
            updatePlayerFocusNavigation()
        }
        player.addListener(object : Player.Listener {
            override fun onTracksChanged(tracks: Tracks) {
                availableAudioTracks = tracks.groups
                    .filter { it.type == C.TRACK_TYPE_AUDIO }
                    .flatMap { group -> (0 until group.length)
                        .filter { group.isTrackSupported(it) || group.isTrackSelected(it) }
                        .map { index -> group to index }
                    }
                hasMultipleAudioTracks = availableAudioTracks.size > 1
                availableSubtitleTracks = tracks.groups
                    .filter { it.type == C.TRACK_TYPE_TEXT }
                    .flatMap { group -> (0 until group.length)
                        .filter { group.isTrackSupported(it) || group.isTrackSelected(it) }
                        .map { index -> group to index }
                    }
                if (availableSubtitleTracks.isEmpty()) subtitleButton.text = "Legendas ▾"
                updateTrackButtons(view.isControllerFullyVisible)
            }
        })
        // Os botões sobrepostos não fazem parte do cronômetro de foco do Media3.
        // Enquanto Áudio, Legendas, Pular ou Próximo episódio estiver focado, mantenha os controles
        // visíveis; ao sair, restaure os 6 segundos normais de auto-ocultação.
        fun updateOverlayButtonFocusTimeout() {
            val overlayButtonHasFocus = audioButton.hasFocus() || subtitleButton.hasFocus() ||
                skipButton.hasFocus() || nextEpisodeButton.hasFocus() || cancelNextEpisodeButton.hasFocus()
            view.setControllerShowTimeoutMs(PlayerControllerFocusPolicy.timeoutMs(overlayButtonHasFocus))
            if (overlayButtonHasFocus) view.showController()
        }
        listOf(audioButton, subtitleButton, skipButton, nextEpisodeButton, cancelNextEpisodeButton).forEach { button ->
            button.setOnFocusChangeListener { _, _ -> updateOverlayButtonFocusTimeout() }
        }
        view.setControllerVisibilityListener(PlayerView.ControllerVisibilityListener { visibility ->
            val overlayButtonHasFocus = audioButton.hasFocus() || subtitleButton.hasFocus() ||
                skipButton.hasFocus() || nextEpisodeButton.hasFocus() || cancelNextEpisodeButton.hasFocus()
            if (visibility != View.VISIBLE && overlayButtonHasFocus) view.showController()
            updateTrackButtons(visibility == View.VISIBLE)
        })

        // Pular abertura, recapitulação e créditos: um botão sobre o vídeo enquanto o trecho toca.
        val skips = parseSkips(args.segments)
        var shownEnd: Double? = null
        skipButton.setOnClickListener {
            val end = skipButton.tag as? Double
            if (end != null) player.seekTo((end * 1000).toLong())
            val returnFocus = skipButton.hasFocus()
            skipButton.visibility = View.GONE
            shownEnd = null
            updatePlayerFocusNavigation()
            if (returnFocus) view.findViewById<View>(androidx.media3.ui.R.id.exo_play_pause)?.requestFocus()
        }
        val skipTick = object : Runnable {
            override fun run() {
                val t = player.currentPosition / 1000.0
                val seg = skips.firstOrNull { t >= it.start && t < it.end - 0.5 }
                val end = seg?.end
                if (end != shownEnd) {
                    shownEnd = end
                    if (seg == null) {
                        val returnFocus = skipButton.hasFocus()
                        skipButton.visibility = View.GONE
                        if (returnFocus) {
                            view.findViewById<View>(androidx.media3.ui.R.id.exo_play_pause)?.requestFocus()
                        }
                    } else {
                        skipButton.text = seg.label + " ▶"
                        skipButton.tag = seg.end
                        skipButton.visibility = View.VISIBLE
                    }
                    updatePlayerFocusNavigation()
                }
                handler.postDelayed(this, 500)
            }
        }
        val nextEpisodeCountdownTick = object : Runnable {
            override fun run() {
                if (!nextEpisodeCountdownActive || nextEpisodeCountdownCancelled) return
                countdownSecondsRemaining--
                if (NextEpisodeAutoPlayPolicy.shouldStart(
                        hasNextEpisode = NextEpisodeAutoPlayPolicy.hasNextEpisode(nextEpisodeLabel),
                        cancelled = nextEpisodeCountdownCancelled,
                        secondsRemaining = countdownSecondsRemaining,
                    )
                ) {
                    nextEpisodeCountdownActive = false
                    autoNextRequested = true
                    nextEpisodeStatus.text = "Abrindo próximo episódio…"
                    dialog.dismiss()
                    return
                }
                nextEpisodeStatus.text =
                    "Próximo: $nextEpisodeLabel em ${countdownSecondsRemaining}s"
                handler.postDelayed(this, 1_000)
            }
        }
        fun showPlaybackFinishedWithoutNextEpisode() {
            finishedWithoutNextEpisode = true
            nextEpisodeStatus.text = "Episódio concluído · Não há próximo episódio disponível"
            nextEpisodeButton.text = "Fechar player"
            cancelNextEpisodeButton.visibility = View.GONE
            nextEpisodePanel.visibility = View.VISIBLE
            updatePlayerFocusNavigation()
            view.showController()
            nextEpisodeButton.requestFocus()
        }
        fun startNextEpisodeCountdown() {
            if (!NextEpisodeAutoPlayPolicy.hasNextEpisode(nextEpisodeLabel) ||
                nextEpisodeCountdownActive || nextEpisodeCountdownCancelled
            ) return
            finishedWithoutNextEpisode = false
            nextEpisodeButton.text = "Reproduzir agora"
            cancelNextEpisodeButton.visibility = View.VISIBLE
            nextEpisodeCountdownActive = true
            countdownSecondsRemaining = NextEpisodeAutoPlayPolicy.COUNTDOWN_SECONDS
            nextEpisodeStatus.text =
                "Próximo: $nextEpisodeLabel em ${countdownSecondsRemaining}s"
            nextEpisodePanel.visibility = View.VISIBLE
            updatePlayerFocusNavigation()
            view.showController()
            nextEpisodeButton.requestFocus()
            handler.postDelayed(nextEpisodeCountdownTick, 1_000)
        }
        nextEpisodeButton.setOnClickListener {
            if (finishedWithoutNextEpisode) {
                dialog.dismiss()
                return@setOnClickListener
            }
            if (!nextEpisodeCountdownActive || nextEpisodeCountdownCancelled) return@setOnClickListener
            nextEpisodeCountdownActive = false
            autoNextRequested = true
            handler.removeCallbacks(nextEpisodeCountdownTick)
            nextEpisodeStatus.text = "Abrindo próximo episódio…"
            dialog.dismiss()
        }
        fun cancelNextEpisodeCountdown() {
            val returnFocus = nextEpisodeButton.hasFocus() || cancelNextEpisodeButton.hasFocus()
            nextEpisodeCountdownCancelled = true
            nextEpisodeCountdownActive = false
            handler.removeCallbacks(nextEpisodeCountdownTick)
            nextEpisodePanel.visibility = View.GONE
            updatePlayerFocusNavigation()
            if (returnFocus) {
                view.findViewById<View>(androidx.media3.ui.R.id.exo_play_pause)?.requestFocus()
            }
        }
        cancelNextEpisodeButton.setOnClickListener {
            cancelNextEpisodeCountdown()
        }
        dialog.setOnKeyListener { _, keyCode, event ->
            val dismissCountdown = keyCode == KeyEvent.KEYCODE_BACK &&
                NextEpisodeAutoPlayPolicy.shouldDismissCountdownOnBack(
                    panelVisible = nextEpisodePanel.visibility == View.VISIBLE,
                    countdownActive = nextEpisodeCountdownActive && !nextEpisodeCountdownCancelled,
                )
            if (!dismissCountdown) {
                false
            } else {
                // Consome DOWN e UP: um único Voltar fecha somente o aviso, sem
                // deixar o Dialog dispensar o player que já terminou de tocar.
                if (event.action == KeyEvent.ACTION_UP) cancelNextEpisodeCountdown()
                true
            }
        }
        player.addListener(object : Player.Listener {
            override fun onPlaybackStateChanged(playbackState: Int) {
                if (playbackState == Player.STATE_ENDED) {
                    playbackEnded = true
                    if (NextEpisodeAutoPlayPolicy.shouldShowCompletionExit(
                            playbackEnded = true,
                            hasNextEpisode = NextEpisodeAutoPlayPolicy.hasNextEpisode(nextEpisodeLabel),
                        )
                    ) {
                        showPlaybackFinishedWithoutNextEpisode()
                    } else {
                        startNextEpisodeCountdown()
                    }
                }
            }
        })
        val frame = FrameLayout(act)
        frame.addView(view, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        val buttonParams = FrameLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.TOP or Gravity.START)
        val margin = (24 * density).toInt()
        buttonParams.setMargins(margin, margin, 0, 0)
        frame.addView(skipButton, buttonParams)
        val audioButtonParams = FrameLayout.LayoutParams(
            ViewGroup.LayoutParams.WRAP_CONTENT,
            ViewGroup.LayoutParams.WRAP_CONTENT,
            Gravity.TOP or Gravity.END
        )
        audioButtonParams.setMargins(0, margin, margin, 0)
        frame.addView(audioButton, audioButtonParams)
        val subtitleButtonParams = FrameLayout.LayoutParams(
            ViewGroup.LayoutParams.WRAP_CONTENT,
            ViewGroup.LayoutParams.WRAP_CONTENT,
            Gravity.TOP or Gravity.END
        )
        subtitleButtonParams.setMargins(0, margin + (56 * density).toInt(), margin, 0)
        frame.addView(subtitleButton, subtitleButtonParams)
        val nextEpisodePanelParams = FrameLayout.LayoutParams(
            ViewGroup.LayoutParams.WRAP_CONTENT,
            ViewGroup.LayoutParams.WRAP_CONTENT,
            Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL
        )
        nextEpisodePanelParams.setMargins(margin, 0, margin, (104 * density).toInt())
        frame.addView(nextEpisodePanel, nextEpisodePanelParams)
        dialog.setContentView(frame, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))

        var errorMessage: String? = null
        val subtitleFiles = mutableListOf<File>()
        var answered = false
        fun answer() {
            if (answered) return
            answered = true
            handler.removeCallbacks(skipTick)
            handler.removeCallbacks(nextEpisodeCountdownTick)
            val position = player.currentPosition.coerceAtLeast(0L)
            val duration = player.duration.takeIf { it > 0L } ?: 0L
            view.player = null
            player.release()
            subtitleFiles.forEach { it.delete() }
            subtitleFiles.clear()
            if (errorMessage != null) invoke.reject(errorMessage!!)
            else invoke.resolve(JSObject().apply {
                put("positionMs", position)
                put("durationMs", duration)
                put("ended", playbackEnded)
                put("autoNext", autoNextRequested)
            })
        }
        player.addListener(object : Player.Listener {
            override fun onPlayerError(error: PlaybackException) {
                errorMessage = friendlyMessage(error)
                dialog.dismiss()
            }
        })
        dialog.setOnDismissListener { answer() }
        dialog.show()
        view.requestFocus()
        handler.post(skipTick)

        val item = MediaItem.Builder().setUri(args.url)
        val subtitleConfigurations = mutableListOf<MediaItem.SubtitleConfiguration>()
        try {
            val subtitleArray = args.subtitles?.let { JSONArray(it) } ?: JSONArray()
            for (index in 0 until subtitleArray.length()) {
                val subtitle = subtitleArray.optJSONObject(index) ?: continue
                val encoded = subtitle.optString("data")
                if (encoded.isBlank()) continue
                val bytes = Base64.decode(encoded, Base64.DEFAULT)
                if (bytes.isEmpty()) continue
                val file = File.createTempFile("cinehub-subtitle-", ".vtt", act.cacheDir)
                file.writeBytes(bytes)
                subtitleFiles.add(file)
                val language = subtitle.optString("language", "und").takeIf { it.isNotBlank() } ?: "und"
                val label = subtitle.optString("label", "Legenda " + (index + 1)).takeIf { it.isNotBlank() }
                    ?: "Legenda " + (index + 1)
                val isDefault = subtitle.optBoolean("isDefault", false)
                subtitleConfigurations.add(
                    MediaItem.SubtitleConfiguration.Builder(Uri.fromFile(file))
                        .setMimeType(MimeTypes.TEXT_VTT)
                        .setLanguage(language)
                        .setLabel(label)
                        .setSelectionFlags(if (isDefault) C.SELECTION_FLAG_DEFAULT else 0)
                        .build()
                )
            }
        } catch (e: Exception) {
            android.util.Log.w("CineHubExoPlayer", "Não foi possível preparar uma ou mais legendas externas", e)
        }
        if (subtitleConfigurations.isNotEmpty()) item.setSubtitleConfigurations(subtitleConfigurations)
        player.setMediaItem(item.build())
        if (preferences.getBoolean("subtitlesDisabled", args.subtitlesDisabled)) {
            player.trackSelectionParameters = player.trackSelectionParameters.buildUpon()
                .setTrackTypeDisabled(C.TRACK_TYPE_TEXT, true)
                .build()
            subtitleButton.text = "Legendas: desativadas ▾"
        }
        if (args.startMs > 0) player.seekTo(args.startMs)
        player.prepare()
        player.playWhenReady = true
    }

    private fun parseSkips(json: String?): List<Skip> {
        if (json.isNullOrBlank()) return emptyList()
        return try {
            val arr = JSONArray(json)
            (0 until arr.length()).map { i ->
                val o = arr.getJSONObject(i)
                Skip(o.getString("type"), o.getDouble("start"), o.getDouble("end"))
            }
        } catch (e: Exception) {
            emptyList()
        }
    }

    private fun friendlyMessage(error: PlaybackException): String {
        val detail = when (error.errorCode) {
            PlaybackException.ERROR_CODE_IO_CLEARTEXT_NOT_PERMITTED -> "conexão sem criptografia bloqueada pelo sistema"
            PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_FAILED,
            PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_TIMEOUT -> "falha de rede ao baixar o vídeo. Verifique a conexão"
            PlaybackException.ERROR_CODE_IO_BAD_HTTP_STATUS -> "o servidor recusou o vídeo"
            PlaybackException.ERROR_CODE_IO_FILE_NOT_FOUND -> "o vídeo não foi encontrado no servidor"
            PlaybackException.ERROR_CODE_PARSING_CONTAINER_UNSUPPORTED,
            PlaybackException.ERROR_CODE_PARSING_CONTAINER_MALFORMED -> "formato de contêiner não suportado ou arquivo danificado"
            PlaybackException.ERROR_CODE_DECODER_INIT_FAILED,
            PlaybackException.ERROR_CODE_DECODING_FAILED,
            PlaybackException.ERROR_CODE_DECODING_FORMAT_UNSUPPORTED,
            PlaybackException.ERROR_CODE_DECODING_FORMAT_EXCEEDS_CAPABILITIES -> "codec sem suporte neste aparelho"
            else -> "erro de reprodução"
        }
        return "O Android não conseguiu tocar: $detail (${error.errorCodeName})"
    }
}
