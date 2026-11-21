package com.cinehub.exoplayer

/**
 * Interpreta o resultado JavaScript retornado pelo WebView ao pressionar Voltar.
 *
 * O callback de evaluateJavascript retorna o número JavaScript como texto:
 * "1" quando a página consumiu o evento e "0" quando o Android deve continuar.
 */
internal object NativeBackNavigationPolicy {
    fun isHandledByWebView(result: String?): Boolean = result == "1"
}
