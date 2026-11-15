# CineHub Addon

Addon para o [Stremio](https://www.stremio.com/) escrito em Rust. Ele expõe a sua biblioteca local de filmes e séries pelo protocolo oficial de addons, com metadados e legendas em português.

Ele não busca, não baixa e não distribui conteúdo. Só serve arquivos que você já tem no seu computador ou NAS.

## Recursos

- Protocolo de addons do Stremio: `manifest`, `catalog` (com busca), `meta` e `stream`
- Reconhece filmes (`A Origem (2010) 1080p.mkv`) e episódios (`Dark.S01E02.mkv`)
- Detecta a qualidade no nome do arquivo (2160p, 1080p, 720p, 480p)
- Legendas em português ao lado do vídeo (`Dark.S01E02.pt-BR.srt`, `.vtt`, `.por.srt`)
- Suporte a `Range`, para o player avançar e voltar
- CORS liberado, para o Stremio Web conseguir consultar o addon

## Requisitos

- Rust estável (1.75 ou superior) com `cargo`

## Executar

```bash
# coloque seus arquivos em ./media (ou defina MEDIA_DIR)
MEDIA_DIR=/caminho/para/sua/biblioteca PORT=7000 cargo run --release
```

Variáveis de ambiente:

| Variável    | Padrão                    | Descrição                                                                 |
|-------------|---------------------------|---------------------------------------------------------------------------|
| `MEDIA_DIR` | `./media`                 | Pasta com os vídeos (varredura recursiva)                                 |
| `PORT`      | `7000`                    | Porta HTTP do addon                                                       |
| `BASE_URL`  | `http://127.0.0.1:$PORT`  | URL pela qual o Stremio acessa o addon (ex.: `http://192.168.0.10:7000`)  |

## Instalar no Stremio

1. Confirme que o addon está rodando. O log mostra a URL do manifesto.
2. Abra o Stremio, vá em **Addons** e escolha a opção de instalar por URL.
3. Cole `http://<seu-ip>:7000/manifest.json` e confirme.

Se você usa o Stremio Web, ele pode bloquear `http://` por ser conteúdo misto. Nesse caso, use o app de desktop ou exponha o addon por HTTPS (por exemplo, com um proxy reverso).

## Nomes de arquivo

- Filmes: `Titulo (Ano) qualidade.ext` ou `Titulo.Ano.qualidade.ext`
- Séries: `Titulo.S01E02.qualidade.ext` ou `Titulo - s01e02.ext`
- Legendas: mesmo começo do nome do vídeo, com marcador de português no nome: `.pt`, `.pt-BR`, `.por`, `.pob`

## Estrutura

```
src/main.rs      rotas HTTP do protocolo do Stremio e entrega de arquivos
src/library.rs   varredura da pasta, interpretação de nomes e legendas
```

## Testes

```bash
cargo test
```

## Licença

MIT
