// Bearer token comes from MSAL (silent acquisition against the API scope),
// registered here by AuthProvider — never read from/written to localStorage
// or sessionStorage by this module. MSAL manages its own token cache
// (configured for sessionStorage) internally; that is identity-cache
// housekeeping, not application data, and this module never touches it.

type TokenProvider = () => Promise<string | null>

let tokenProvider: TokenProvider | null = null
let currentUserId: string | null = null

export function setTokenProvider(provider: TokenProvider | null) {
  tokenProvider = provider
}

export function setCurrentUserId(id: string | null) {
  currentUserId = id
}

export function getCurrentUserId(): string | null {
  return currentUserId
}

async function getBearerToken(): Promise<string | null> {
  if (!tokenProvider) return null
  try {
    return await tokenProvider()
  } catch {
    return null
  }
}

async function request(path: string, init: RequestInit = {}) {
  const token = await getBearerToken()
  const response = await fetch(`/api/v1${path}`, {
    ...init,
    headers: { Accept: 'application/json', ...(init.body ? { 'Content-Type': 'application/json' } : {}), ...(token ? { Authorization: `Bearer ${token}` } : {}), ...(init.headers || {}) },
  })
  const text = await response.text()
  let data: any = null
  try { data = text ? JSON.parse(text) : null } catch { data = text }
  if (!response.ok) {
    const error: any = new Error(typeof data === 'string' ? data : data?.detail || data?.message || `Request failed (${response.status})`)
    error.status = response.status; error.data = data; throw error
  }
  return data
}

export const pasApi = {
  products: () => request('/products'), customers: () => request('/customers'),
  createCustomer: (body:any) => request('/customers', { method:'POST', body:JSON.stringify(body) }),
  estimate: (body:any) => request('/pricing/estimate', { method:'POST', body:JSON.stringify(body) }),
  rate: (body:any) => request('/rating/quote', { method:'POST', body:JSON.stringify(body) }),
  quotes: () => request('/quotes'), createQuote: (body:any) => request('/quotes', { method:'POST', body:JSON.stringify(body) }),
  quote: (id:string) => request(`/quotes/${encodeURIComponent(id)}`),
  bind: (id:string) => request(`/quotes/${encodeURIComponent(id)}/bind`, { method:'POST' }),
  issueQuote: (id:string) => request(`/quotes/${encodeURIComponent(id)}/issue`, { method:'POST' }),
  timeline: (id:string) => request(`/quotes/${encodeURIComponent(id)}/timeline`),
  policies: () => request('/policies'), policy: (id:string) => request(`/policies/${encodeURIComponent(id)}`),
  versions: (id:string) => request(`/policies/${encodeURIComponent(id)}/versions`),
  endorse: (body:any) => request('/pas/endorse', { method:'POST', body:JSON.stringify(body) }),
  cancel: (body:any) => request('/pas/cancel', { method:'POST', body:JSON.stringify(body) }),
  reinstate: (body:any) => request('/pas/reinstate', { method:'POST', body:JSON.stringify(body) }),
  agents: () => request('/agents'),
  createAgent: (body:any) => request('/agents', { method:'POST', body:JSON.stringify(body) }),
  document: async (id:string) => {
    const token = await getBearerToken()
    const r=await fetch(`/api/v1/policies/${encodeURIComponent(id)}/document`,{headers:token?{Authorization:`Bearer ${token}`}:{}})
    if(!r.ok) throw new Error(`Document request failed (${r.status})`); return r.blob()
  },
}

export function listOf(body:any,key:string){ return Array.isArray(body)?body:(body?.[key]||body?.items||[]) }
export function messageOf(error:any,fallback:string){ return error?.message || fallback }
export function isSkeleton(error:any){ return error?.status===422 && JSON.stringify(error?.data ?? error?.message).includes('pas:skeleton:v1') }
