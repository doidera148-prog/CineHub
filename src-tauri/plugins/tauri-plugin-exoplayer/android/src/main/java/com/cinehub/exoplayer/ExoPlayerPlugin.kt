package com.cinehub.exoplayer

import android.app.Activity
import android.app.Dialog
import android.view.ViewGroup
import android.widget.FrameLayout
import androidx.media3.common.MediaItem
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.exoplayer.DefaultRenderersFactory
import androidx.media3.exoplayer.ExoPlayer
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
}

/**
 * Abre o vídeo em tela cheia com ExoPlayer (Media3), por cima da WebView.
 * O comando só responde quando o usuário fecha o player (voltar ou dispensar),
 * com a posição final, para a interface salvar o progresso.
 * Se o ExoPlayer não conseguir tocar, responde com erro e a interface usa o player web.
 */
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
        act.runOnUiThread { openPlayer(invoke, args) }
    }

    private fun openPlayer(invoke: Invoke, args: PlayArgs) {
        // Decodificador por hardware quando existir; se falhar, cai para o de software.
        val renderers = DefaultRenderersFactory(act).setEnableDecoderFallback(true)
        val player = ExoPlayer.Builder(act, renderers).build()

        val view = PlayerView(act).apply {
            this.player = player
            useController = true
            keepScreenOn = true
        }
        val dialog = Dialog(act, android.R.style.Theme_Black_NoTitleBar_Fullscreen)
        dialog.setContentView(view, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))

        var errorMessage: String? = null
        var answered = false

        fun answer() {
            if (answered) return
            answered = true
            val position = player.currentPosition
            val duration = player.duration
            player.release()
            val err = errorMessage
            if (err != null) {
                invoke.reject(err)
            } else {
                val result = JSObject()
                result.put("positionMs", position)
                result.put("durationMs", if (duration > 0) duration else 0L)
                invoke.resolve(result)
            }
        }

        player.addListener(object : Player.Listener {
            override fun onPlayerError(error: PlaybackException) {
                errorMessage = "ExoPlayer não conseguiu tocar: ${error.errorCodeName}"
                dialog.dismiss()
            }
        })

        dialog.setOnDismissListener { answer() }
        dialog.show()

        player.setMediaItem(MediaItem.fromUri(args.url))
        if (args.startMs > 0) player.seekTo(args.startMs)
        player.prepare()
        player.playWhenReady = true
    }
}
