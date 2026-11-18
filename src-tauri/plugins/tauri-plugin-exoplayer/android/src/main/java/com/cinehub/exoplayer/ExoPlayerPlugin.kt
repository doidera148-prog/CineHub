package com.cinehub.exoplayer

import android.app.Activity
import android.app.AlertDialog
import android.app.Dialog
import android.view.ViewGroup
import android.widget.FrameLayout
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
    /** Trechos para pular, como JSON: [{"type":"intro","start":5,"end":90}, ...] (opcional). */
    var segments: String? = null
}

/** Validação isolada para permitir testes unitários sem iniciar a Activity/player. */
internal object ExoPlayerUrlValidator {
    fun isPlayableUrl(raw: String): Boolean {
        return try {
            val uri = Uri.parse(raw.trim())
            (uri.scheme.equals("http", ignoreCase = true) ||
                uri.scheme.equals("https", ignoreCase = true)) &&
                !uri.host.isNullOrBlank()
        } catch (_: Exception) {
            false
        }
    }
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

/** Player Android/Android TV em tela cheia, com controles Media3 acessíveis por toque e D-pad. */
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
        // Prioriza português brasileiro; usa português genérico como fallback.
        player.trackSelectionParameters = player.trackSelectionParameters.buildUpon()
            .setPreferredAudioLanguages("pt-BR", "pt")
            .setPreferredTextLanguages("pt-BR", "pt")
            .build()

        val view = PlayerView(act).apply {
            this.player = player
            useController = true
            controllerShowTimeoutMs = 6000
            resizeMode = AspectRatioFrameLayout.RESIZE_MODE_FIT
            keepScreenOn = true
            isFocusable = true
            isFocusableInTouchMode = true
            descendantFocusability = ViewGroup.FOCUS_AFTER_DESCENDANTS
            setShowBuffering(PlayerView.SHOW_BUFFERING_WHEN_PLAYING)
        }
        val dialog = Dialog(act, android.R.style.Theme_Black_NoTitleBar_Fullscreen)

        // Permite trocar manualmente a faixa de áudio em arquivos com múltiplos idiomas.
        val audioButton = Button(act).apply {
            text = "Áudio"
            isAllCaps = false
            visibility = View.GONE
            isFocusable = true
            isFocusableInTouchMode = true
            setOnClickListener {
                androidx.media3.ui.TrackSelectionDialogBuilder(
                    act,
                    "Faixa de áudio",
                    player,
                    C.TRACK_TYPE_AUDIO
                ).build().show()
            }
        }
        // Seletor de legendas: lista todas as faixas e oferece a opção Desativadas.
        var availableSubtitleTracks: List<Pair<Tracks.Group, Int>> = emptyList()
        lateinit var subtitleButton: Button
        subtitleButton = Button(act).apply {
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
                                subtitleButton.text = "Legendas: desativadas ▾"
                            } else {
                                val (group, trackIndex) = tracks[which - 1]
                                builder.setTrackTypeDisabled(C.TRACK_TYPE_TEXT, false)
                                    .addOverride(TrackSelectionOverride(group.mediaTrackGroup, listOf(trackIndex)))
                                subtitleButton.text = "Legendas: " + choices[which] + " ▾"
                            }
                            player.trackSelectionParameters = builder.build()
                            dialog.dismiss()
                        }.setNegativeButton("Fechar", null).show()
                }
            }
        }
        var hasMultipleAudioTracks = false
        fun updateTrackButtons(visible: Boolean) {
            audioButton.visibility = if (hasMultipleAudioTracks && visible) View.VISIBLE else View.GONE
            // Botão fica disponível mesmo sem faixa para explicar que a fonte não oferece legenda.
            subtitleButton.visibility = if (visible) View.VISIBLE else View.GONE
        }
        player.addListener(object : Player.Listener {
            override fun onTracksChanged(tracks: Tracks) {
                hasMultipleAudioTracks = tracks.groups
                    .filter { it.type == C.TRACK_TYPE_AUDIO }
                    .sumOf { group -> (0 until group.length).count { group.isTrackSupported(it) } } > 1
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
        view.setControllerVisibilityListener(PlayerView.ControllerVisibilityListener { visibility ->
            updateTrackButtons(visibility == View.VISIBLE)
        })

        // Pular abertura, recapitulação e créditos: um botão sobre o vídeo enquanto o trecho toca.
        val skips = parseSkips(args.segments)
        val handler = Handler(Looper.getMainLooper())
        val density = act.resources.displayMetrics.density
        var shownEnd: Double? = null
        val skipButton = Button(act)
        skipButton.visibility = View.GONE
        skipButton.isAllCaps = false
        skipButton.setOnClickListener {
            val end = skipButton.tag as? Double
            if (end != null) player.seekTo((end * 1000).toLong())
            skipButton.visibility = View.GONE
            shownEnd = null
        }
        val skipTick = object : Runnable {
            override fun run() {
                val t = player.currentPosition / 1000.0
                val seg = skips.firstOrNull { t >= it.start && t < it.end - 0.5 }
                val end = seg?.end
                if (end != shownEnd) {
                    shownEnd = end
                    if (seg == null) {
                        skipButton.visibility = View.GONE
                    } else {
                        skipButton.text = seg.label + " ▶"
                        skipButton.tag = seg.end
                        skipButton.visibility = View.VISIBLE
                    }
                }
                handler.postDelayed(this, 500)
            }
        }
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
        dialog.setContentView(frame, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))

        var errorMessage: String? = null
        val subtitleFiles = mutableListOf<File>()
        var answered = false
        fun answer() {
            if (answered) return
            answered = true
            handler.removeCallbacks(skipTick)
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
        if (args.subtitlesDisabled) {
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
