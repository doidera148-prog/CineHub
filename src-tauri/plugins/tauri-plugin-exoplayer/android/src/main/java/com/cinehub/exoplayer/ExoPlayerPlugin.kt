package com.cinehub.exoplayer

import android.app.Activity
import android.app.Dialog
import android.view.ViewGroup
import android.widget.FrameLayout
import android.net.Uri
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MimeTypes
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
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
    /** Legenda VTT como data: URI (opcional). */
    var subtitle: String? = null
    /** Trechos para pular, como JSON: [{"type":"intro","start":5,"end":90}, ...] (opcional). */
    var segments: String? = null
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
        if (!isPlayableUrl(args.url)) {
            invoke.reject("endereço de vídeo inválido")
            return
        }
        act.runOnUiThread { openPlayer(invoke, args) }
    }

    /** Aceita HTTP(S) com host válido, preservando streams locais de torrent e LAN. */
    private fun isPlayableUrl(raw: String): Boolean {
        return try {
            val uri = Uri.parse(raw.trim())
            (uri.scheme.equals("http", ignoreCase = true) ||
                uri.scheme.equals("https", ignoreCase = true)) &&
                !uri.host.isNullOrBlank()
        } catch (_: Exception) {
            false
        }
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
        // Controle direto de legendas: não depende do diálogo genérico de seleção.
        // Isso permite desligar inclusive uma faixa externa que foi selecionada automaticamente.
        var subtitlesDisabled = false
        val subtitleButton = Button(act).apply {
            text = "Desativar legendas"
            isAllCaps = false
            visibility = View.GONE
            isFocusable = true
            isFocusableInTouchMode = true
            setOnClickListener {
                subtitlesDisabled = !subtitlesDisabled
                player.trackSelectionParameters = player.trackSelectionParameters.buildUpon()
                    .setTrackTypeDisabled(C.TRACK_TYPE_TEXT, subtitlesDisabled)
                    .build()
                text = if (subtitlesDisabled) "Ativar legendas" else "Desativar legendas"
            }
        }
        var hasMultipleAudioTracks = false
        var hasSubtitleTracks = false
        fun updateTrackButtons(visible: Boolean) {
            audioButton.visibility = if (hasMultipleAudioTracks && visible) View.VISIBLE else View.GONE
            subtitleButton.visibility = if (hasSubtitleTracks && visible) View.VISIBLE else View.GONE
        }
        player.addListener(object : Player.Listener {
            override fun onTracksChanged(tracks: androidx.media3.common.Tracks) {
                hasMultipleAudioTracks = tracks.groups
                    .filter { it.type == C.TRACK_TYPE_AUDIO }
                    .sumOf { group ->
                        (0 until group.length).count { group.isTrackSupported(it) }
                    } > 1
                // Considera as faixas de texto presentes mesmo quando o renderer não
                // consegue classificá-las como suportadas antes da preparação completa.
                hasSubtitleTracks = tracks.groups
                    .any { it.type == C.TRACK_TYPE_TEXT && it.length > 0 }
                // Os seletores só aparecem junto com os controles do player.
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
        var subtitleFile: File? = null
        var answered = false
        fun answer() {
            if (answered) return
            answered = true
            handler.removeCallbacks(skipTick)
            val position = player.currentPosition.coerceAtLeast(0L)
            val duration = player.duration.takeIf { it > 0L } ?: 0L
            view.player = null
            player.release()
            subtitleFile?.delete()
            subtitleFile = null
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
        args.subtitle?.takeIf { it.startsWith("data:text/vtt;base64,") }?.let { dataUri ->
            // Evita depender do suporte a data: URI no DataSource do Media3 em diferentes
            // versões/aparelhos Android. O arquivo temporário é removido ao fechar o player.
            try {
                val encoded = dataUri.substringAfter(',', "")
                val bytes = Base64.decode(encoded, Base64.DEFAULT)
                if (bytes.isNotEmpty()) {
                    val file = File.createTempFile("cinehub-subtitle-", ".vtt", act.cacheDir)
                    file.writeBytes(bytes)
                    subtitleFile = file
                    item.setSubtitleConfigurations(listOf(
                        MediaItem.SubtitleConfiguration.Builder(Uri.fromFile(file))
                            .setMimeType(MimeTypes.TEXT_VTT)
                            .setLanguage("pt")
                            .setSelectionFlags(C.SELECTION_FLAG_DEFAULT)
                            .build()
                    ))
                }
            } catch (e: Exception) {
                subtitleFile?.delete()
                subtitleFile = null
                android.util.Log.w("CineHubExoPlayer", "Não foi possível preparar a legenda externa", e)
            }
        }
        player.setMediaItem(item.build())
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
