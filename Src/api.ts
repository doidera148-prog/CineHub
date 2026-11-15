import type { Media, MediaType } from './types'

const KEY = import.meta.env.VITE_TMDB_API_KEY as string | undefined
const BASE = 'https://api.themoviedb.org/3'
const demo: Media[] = [
  { id: 1, title: 'Duna: Parte Dois', overview: 'Paul Atreides se une a Chani e aos Fremen enquanto busca vingança contra os conspiradores que destruíram sua família.', poster_path: null, backdrop_path: null, vote_average: 8.2, release_date: '2024-02-27', media_type: 'movie' },
  { id: 2, title: 'Oppenheimer', overview: 'A história do físico J. Robert Oppenheimer e seu papel no desenvolvimento da bomba atômica.', poster_path: null, backdrop_path: null, vote_average: 8.1, release_date: '2023-07-19', media_type: 'movie' },
  { id: 3, title: 'Interestelar', overview: 'Uma equipe de exploradores viaja além desta galáxia para descobrir se a humanidade tem futuro entre as estrelas.', poster_path: null, backdrop_path: null, vote_average: 8.7, release_date: '2014-11-05', media_type: 'movie' },
  { id: 4, title: 'The Last of Us', overview: 'Vinte anos após a queda da civilização, Joel é contratado para tirar Ellie de uma zona de quarentena.', poster_path: null, backdrop_path: null, vote_average: 8.6, first_air_date: '2023-01-15', media_type: 'tv' },
  { id: 5, title: 'Arcane', overview: 'Em meio ao conflito entre cidades-estado, duas irmãs seguem caminhos opostos em uma história de poder e consequências.', poster_path: null, backdrop_path: null, vote_average: 9.0, first_air_date: '2021-11-06', media_type: 'tv' },
  { id: 6, title: 'Breaking Bad', overview: 'Um professor de química começa a produzir metanfetamina após receber um diagnóstico que muda sua vida.', poster_path: null, backdrop_path: null, vote_average: 9.5, first_air_date: '2008-01-20', media_type: 'tv' },
]
export const imageUrl = (path: string | null, size = 'w500') => path ? `https://image.tmdb.org/t/p/${size}${path}` : ''
export const hasApiKey = Boolean(KEY && KEY !== 'cole_sua_chave_aqui')

async function request(path: string): Promise<any> {
  if (!hasApiKey) throw new Error('Configure a chave TMDB para carregar o catálogo atualizado.')
  const response = await fetch(`${BASE}${path}${path.includes('?') ? '&' : '?'}api_key=${KEY}&language=pt-BR`)
  if (!response.ok) throw new Error('Não foi possível carregar o catálogo.')
  return response.json()
}
function normalize(items: any[], type: MediaType): Media[] {
  return items.map(item => ({ ...item, title: item.title || item.name || 'Sem título', media_type: type }))
}
export async function getTrending(): Promise<Media[]> {
  const data = await request('/trending/all/week')
  return (data.results as any[]).filter(x => x.media_type !== 'person').map(x => ({ ...x, title: x.title || x.name }))
}
export async function getPopular(type: MediaType): Promise<Media[]> {
  const data = await request(`/${type}/popular`)
  return normalize(data.results, type)
}
export async function searchMedia(query: string): Promise<Media[]> {
  const data = await request(`/search/multi?query=${encodeURIComponent(query)}`)
  return (data.results as any[]).filter(x => x.media_type === 'movie' || x.media_type === 'tv').map(x => ({ ...x, title: x.title || x.name }))
}
export async function getDetails(item: Media): Promise<any> {
  const type = item.media_type || 'movie'
  return request(`/${type}/${item.id}`)
}
export const demoItems = demo
