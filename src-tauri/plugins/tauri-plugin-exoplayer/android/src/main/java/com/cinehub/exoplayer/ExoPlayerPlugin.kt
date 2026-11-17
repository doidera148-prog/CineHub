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
import androidx.media3.exoplayer.DefaultRenderersFactory
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.AspectRatioFrameLayout
import androidx.media3.ui.PlayerView
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class PlayArgs {
    lateinit var url: String
    var startMs: Long = 0
    /** Legenda VTT como data: URI (opcional). */
    var subtitle: String? = null
}

/** Player Android/Android TV em tela cheia, com controles Media3 acessíveis por toque e D-pad. */
@TauriPlugin
class ExoPlayerPlugin(private val act: Activity) : Plugin(act) {
    @Command
    fun play(invoke: Invoke) {
        val args = try {
            invoke.parseArgs(PlayArgs::class.java)
        } catch (e: Exception) {
            invoke.reject("argumentos inválidos para o player nativo")
            return
        }
        if (!(args.url.startsWith("https://") || args.url.startsWith("http://"))) {
            invoke.reject("endereço de vídeo inválido")
            return
        }
        act.runOnUiThread { openPlayer(invoke, args) }
    }

    private fun openPlayer(invoke: Invoke, args: PlayArgs) {
        val renderers = DefaultRenderersFactory(act)
            .setEnableDecoderFallback(true)
            .setExtensionRendererMode(DefaultRenderersFactory.EXTENSION_RENDERER_MODE_ON)
        val player = ExoPlayer.Builder(act, renderers).build()
        player.trackSelectionParameters = player.trackSelectionParameters.buildUpon()
            .setPreferredAudioLanguage("pt")
            .setPreferredTextLanguage("pt")
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
        dialog.setContentView(view, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))

        var errorMessage: String? = null
        var answered = false
        fun answer() {
            if (answered) return
            answered = true
            val position = player.currentPosition.coerceAtLeast(0L)
            val duration = player.duration.takeIf { it > 0L } ?: 0L
            view.player = null
            player.release()
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

        val item = MediaItem.Builder().setUri(args.url)
        args.subtitle?.takeIf { it.startsWith("data:text/vtt;base64,") }?.let { uri ->
            item.setSubtitleConfigurations(listOf(
                MediaItem.SubtitleConfiguration.Builder(Uri.parse(uri))
                    .setMimeType(MimeTypes.TEXT_VTT)
                    .setLanguage("pt")
                    .setSelectionFlags(C.SELECTION_FLAG_DEFAULT)
                    .build()
            ))
        }
        player.setMediaItem(item.build())
        if (args.startMs > 0) player.seekTo(args.startMs)
        player.prepare()
        player.playWhenReady = true
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
