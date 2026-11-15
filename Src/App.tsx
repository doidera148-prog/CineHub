import { useEffect, useMemo, useState } from 'react'
import { Bookmark, Check, ChevronRight, Clapperboard, Compass, Film, Heart, Info, Play, Search, Sparkles, Star, Tv, X } from 'lucide-react'
import { demoItems, getDetails, getPopular, getTrending, hasApiKey, imageUrl, searchMedia } from './api'
import type { Media, MediaType } from './types'

type Filter = 'all' | 'movie' | 'tv' | 'favorites'
const genres = ['Todos', 'Ação', 'Drama', 'Ficção científica', 'Comédia', 'Animação']
const fallbackColors = ['#25364b', '#44312f', '#253b42', '#3c304d', '#3e3b2a', '#263d39']

function Poster({ item, index, onClick }: { item: Media; index: number; onClick: () => void }) {
  const [failed, setFailed] = useState(false)
  const poster = imageUrl(item.poster_path)
  return <button className="poster-card" onClick={onClick} aria-label={`Ver detalhes de ${item.title}`}>
    <div className="poster-art" style={{ background: fallbackColors[index % fallbackColors.length] }}>
      {poster && !failed ? <img src={poster} alt={item.title} loading="lazy" onError={() => setFailed(true)} /> : <div className="poster-placeholder"><Film size={30}/><span>{item.title}</span></div>}
      <div className="poster-overlay"><span className="play-circle"><Play size={18} fill="currentColor"/></span></div>
      <span className="rating"><Star size={12} fill="currentColor"/> {item.vote_average?.toFixed(1) || '—'}</span>
    </div>
    <span className="poster-title">{item.title}</span>
    <span className="poster-meta">{(item.release_date || item.first_air_date || '').slice(0,4) || 'Catálogo'} · {item.media_type === 'tv' ? 'Série' : 'Filme'}</span>
  </button>
}

export default function App() {
  const [items, setItems] = useState<Media[]>(demoItems)
  const [active, setActive] = useState<Filter>('all')
  const [query, setQuery] = useState('')
  const [selected, setSelected] = useState<Media | null>(null)
  const [details, setDetails] = useState<any>(null)
  const [loading, setLoading] = useState(false)
  const [favorites, setFavorites] = useState<number[]>(() => {
    try { return JSON.parse(localStorage.getItem('cinehub-favorites') || '[]') } catch { return [] }
  })
  const [notice, setNotice] = useState('')

  useEffect(() => { localStorage.setItem('cinehub-favorites', JSON.stringify(favorites)) }, [favorites])
  useEffect(() => {
    if (!hasApiKey) return
    let alive = true
    Promise.all([getTrending(), getPopular('movie'), getPopular('tv')]).then(([trending, movies, shows]) => {
      if (alive) setItems([...trending, ...movies, ...shows].filter((x, i, a) => a.findIndex(y => y.id === x.id && y.media_type === x.media_type) === i))
    }).catch(() => { if (alive) setNotice('Exibindo catálogo de demonstração. Configure sua chave TMDB para acessar o catálogo atualizado.') })
    return () => { alive = false }
  }, [])
  useEffect(() => {
    if (!selected) { setDetails(null); return }
    setDetails(null)
    getDetails(selected).then(setDetails).catch(() => setDetails(null))
  }, [selected])
  useEffect(() => {
    const timer = window.setTimeout(async () => {
      if (!query.trim() || !hasApiKey) return
      setLoading(true)
      try { setItems(await searchMedia(query.trim())) } catch { setNotice('Não foi possível realizar a busca. Verifique sua chave TMDB.') }
      finally { setLoading(false) }
    }, 400)
    return () => window.clearTimeout(timer)
  }, [query])

  const visible = useMemo(() => {
    let list = items
    if (active === 'movie') list = list.filter(x => x.media_type === 'movie')
    if (active === 'tv') list = list.filter(x => x.media_type === 'tv')
    if (active === 'favorites') list = list.filter(x => favorites.includes(x.id))
    if (query.trim() && !hasApiKey) list = list.filter(x => x.title.toLowerCase().includes(query.toLowerCase()))
    return list
  }, [items, active, favorites, query])
  const featured = items.find(x => x.backdrop_path) || items[0] || demoItems[0]
  const toggleFavorite = (id: number) => setFavorites(old => old.includes(id) ? old.filter(x => x !== id) : [...old, id])
  const filters: { id: Filter; label: string; icon: typeof Compass }[] = [
    { id: 'all', label: 'Explorar', icon: Compass }, { id: 'movie', label: 'Filmes', icon: Clapperboard },
    { id: 'tv', label: 'Séries', icon: Tv }, { id: 'favorites', label: 'Minha lista', icon: Bookmark },
  ]

  return <div className="app-shell">
    <aside className="sidebar">
      <a className="brand" href="#"><span className="brand-mark"><Clapperboard size={21}/></span><span>Cine<span>Hub</span></span></a>
      <div className="side-label">MENU</div>
      <nav>{filters.map(({ id, label, icon: Icon }) => <button key={id} className={`nav-item ${active === id ? 'active' : ''}`} onClick={() => setActive(id)}><Icon size={18}/>{label}{id === 'favorites' && favorites.length > 0 && <small>{favorites.length}</small>}</button>)}</nav>
      <div className="sidebar-bottom"><div className="premium-card"><Sparkles size={18}/><strong>Seu próximo favorito está aqui.</strong><p>Explore histórias que combinam com você.</p><button onClick={() => { setActive('all'); setQuery('') }}>Explorar catálogo <ChevronRight size={14}/></button></div><span className="side-foot">CINEHUB · VERSÃO 1.0</span></div>
    </aside>

    <main className="main-content">
      <header className="topbar"><div className="mobile-brand"><Clapperboard size={19}/> Cine<span>Hub</span></div><div className="breadcrumb">Descubra <span>/</span> {filters.find(x => x.id === active)?.label}</div><label className="search-box"><Search size={17}/><input value={query} onChange={e => setQuery(e.target.value)} placeholder="Buscar filmes, séries..."/>{query && <button onClick={() => setQuery('')} aria-label="Limpar busca"><X size={15}/></button>}<kbd>⌘ K</kbd></label><div className="avatar">CH</div></header>
      <section className="hero" style={{ backgroundImage: featured.backdrop_path ? `linear-gradient(90deg,#090b10 4%,rgba(9,11,16,.85) 42%,rgba(9,11,16,.12)),linear-gradient(0deg,#090b10,transparent 45%),url(${imageUrl(featured.backdrop_path, 'original')})` : undefined }}>
        <div className="hero-content"><span className="eyebrow"><Sparkles size={13}/> UNIVERSO CINEHUB</span><h1>Histórias que<br/><em>ficam com você.</em></h1><p>Seu espaço para descobrir filmes e séries incríveis. Explore, salve seus favoritos e encontre sua próxima maratona.</p><div className="hero-actions"><button className="primary-btn" onClick={() => setSelected(featured)}><Play size={16} fill="currentColor"/> Explorar destaque</button><button className="secondary-btn" onClick={() => toggleFavorite(featured.id)}><Heart size={16} fill={favorites.includes(featured.id) ? 'currentColor' : 'none'}/> {favorites.includes(featured.id) ? 'Salvo' : 'Minha lista'}</button></div><div className="hero-note"><span className="live-dot"/> CATÁLOGO PARA DESCOBRIR <span className="note-divider"/> ATUALIZADO SEMANALMENTE</div></div>
        <div className="hero-index"><span>01</span><i/> 03</div>
      </section>
      {notice && <div className="notice"><Info size={15}/>{notice}<button onClick={() => setNotice('')}><X size={14}/></button></div>}
      <section className="catalog-section">
        <div className="section-heading"><div><span className="eyebrow muted">SELEÇÃO PARA VOCÊ</span><h2>{query ? `Resultados para “${query}”` : filters.find(x => x.id === active)?.label}</h2><p>{active === 'favorites' ? 'Os títulos que você guardou.' : 'Encontre algo novo para assistir hoje.'}</p></div><div className="result-count">{visible.length} TÍTULOS</div></div>
        <div className="genre-row">{genres.map((genre, i) => <button key={genre} className={i === 0 ? 'genre active' : 'genre'} onClick={() => { if (i === 0) setQuery(''); else setQuery(genre) }}>{genre}</button>)}</div>
        {loading ? <div className="empty-state">Buscando títulos...</div> : visible.length ? <div className="poster-grid">{visible.map((item, index) => <Poster key={`${item.media_type}-${item.id}`} item={item} index={index} onClick={() => setSelected(item)}/>)}</div> : <div className="empty-state"><Film size={30}/><strong>Nenhum título encontrado</strong><span>{active === 'favorites' ? 'Adicione títulos à sua lista para encontrá-los aqui.' : 'Tente outro termo de busca.'}</span><button onClick={() => { setActive('all'); setQuery('') }}>Voltar para explorar</button></div>}
      </section>
      <footer className="footer"><span>© 2026 CINEHUB</span><span>DESCUBRA. SALVE. APROVEITE.</span></footer>
    </main>

    {selected && <div className="modal-backdrop" onClick={() => setSelected(null)}><section className="detail-modal" onClick={e => e.stopPropagation()}>
      <button className="close-modal" onClick={() => setSelected(null)} aria-label="Fechar"><X size={20}/></button>
      <div className="detail-backdrop" style={{ backgroundImage: selected.backdrop_path ? `linear-gradient(0deg,#11141b,transparent),url(${imageUrl(selected.backdrop_path, 'original')})` : undefined }}/>
      <div className="detail-body"><span className="eyebrow muted">{selected.media_type === 'tv' ? 'SÉRIE' : 'FILME'} · CINEHUB</span><h2>{selected.title}</h2><div className="detail-meta"><span><Star size={14} fill="currentColor"/> {selected.vote_average?.toFixed(1) || '—'}</span><span>{(selected.release_date || selected.first_air_date || '').slice(0,4)}</span>{details?.runtime && <span>{details.runtime} min</span>}</div><p>{details?.overview || selected.overview || 'Sinopse não disponível.'}</p><div className="detail-actions"><button className="primary-btn" onClick={() => setNotice('A reprodução deve ser conectada a uma fonte de vídeo autorizada.')}><Play size={16} fill="currentColor"/> Assistir trailer</button><button className="secondary-btn" onClick={() => toggleFavorite(selected.id)}><Heart size={16} fill={favorites.includes(selected.id) ? 'currentColor' : 'none'}/>{favorites.includes(selected.id) ? 'Na minha lista' : 'Adicionar à lista'}</button></div><small className="provider-note">Informações e imagens fornecidas pelo TMDB. Disponibilidade de reprodução não incluída.</small></div>
    </section></div>}
  </div>
}