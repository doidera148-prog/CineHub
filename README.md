# CineHub

Interface web de descoberta de filmes e séries, inspirada em hubs modernos de streaming. O projeto usa React + TypeScript + Vite e a API pública do TMDB para catálogo, capas, sinopses e avaliações.

> CineHub é um projeto independente de descoberta e organização de conteúdo. Não hospeda, distribui ou desbloqueia filmes/séries. A reprodução deve ser conectada apenas a fontes que você tenha autorização para utilizar.

## Recursos

- Catálogo de filmes e séries populares
- Busca por título
- Filtros por filmes, séries e favoritos
- Detalhes com sinopse, nota e data de lançamento
- Favoritos salvos no navegador
- Layout responsivo para desktop e celular
- Conteúdo demonstrativo quando a chave TMDB ainda não foi configurada

## Executar localmente

Requisitos: Node.js 18 ou superior.

```bash
npm install
cp .env.example .env
```

Adicione sua chave de API do TMDB em `.env`:

```env
VITE_TMDB_API_KEY=sua_chave
```

Depois execute:

```bash
npm run dev
```

Abra o endereço exibido pelo Vite (normalmente http://localhost:5173).

## Gerar versão de produção

```bash
npm run build
npm run preview
```

Os arquivos finais ficam em `dist/`.

## Publicar no GitHub

Crie um repositório público chamado `CineHub` no GitHub e, na pasta deste projeto, execute:

```bash
git init
git add .
git commit -m "feat: primeira versão do CineHub"
git branch -M main
git remote add origin https://github.com/SEU_USUARIO/CineHub.git
git push -u origin main
```

Nunca publique sua chave privada ou o arquivo `.env`. O `.gitignore` já exclui esses arquivos.

## API e atribuição

Os dados e imagens do catálogo são fornecidos pelo [The Movie Database (TMDB)](https://www.themoviedb.org/). Este produto usa a API do TMDB, mas não é endossado nem certificado pelo TMDB. Consulte os termos de uso e as regras de atribuição da API antes de publicar.

## Próximas etapas

- Autenticação e perfis
- Listas sincronizadas na nuvem
- Integração de trailers oficiais
- Backend para proteger credenciais e gerenciar provedores autorizados
- Aplicativo Android
