# Integração com Kodi

O CineHub pode controlar uma instância Kodi configurada manualmente pela API JSON-RPC. Isso permite listar addons de vídeo instalados, navegar nos diretórios `plugin://` e solicitar que o próprio Kodi reproduza um item. Os addons Kodi continuam sendo executados pelo Kodi; o CineHub não os instala nem executa o código Python deles.

## Configuração

1. No Kodi, habilite **Permitir controle remoto via HTTP** nas configurações de serviços. Anote a porta HTTP e, de preferência, configure usuário e senha.
2. Verifique o IP local do aparelho que executa Kodi (por exemplo `192.168.1.20`). Não exponha a porta do Kodi diretamente à internet.
3. No CineHub, abra **Addons → Conectar ao Kodi**, informe `http://IP:porta` e, se configurados, usuário e senha.
4. Selecione um addon, navegue pelo catálogo e escolha **Reproduzir no Kodi**. O vídeo será tocado pelo Kodi, não pelo player incorporado do CineHub.

A integração aceita apenas HTTP para IPs privados/loopback explicitamente informados; não faz descoberta ou varredura da rede, recusa nomes DNS, não segue redirecionamentos e não persiste a senha. As respostas JSON-RPC são limitadas a 2 MiB. Use apenas uma instância Kodi que você controla ou tem autorização para acessar e configure a rede/firewall adequadamente.
