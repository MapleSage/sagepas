// Microsoft Entra ID (Azure AD) configuration — read strictly from env.
// No hardcoded client id, no "common"/organizations tenant fallback.
// Missing configuration is a startup error, not a silent degrade.

export class MsalConfigError extends Error {
  constructor(public missing: string[]) {
    super(`Missing required Entra configuration: ${missing.join(', ')}`)
    this.name = 'MsalConfigError'
  }
}

export interface EntraEnvConfig {
  clientId: string
  tenantId: string
  apiScope: string
  redirectUri: string
}

function readEnv(name: string): string {
  const value = (import.meta.env as Record<string, string | undefined>)[name]
  return value?.trim() ?? ''
}

export function getEntraConfig(): EntraEnvConfig {
  const clientId = readEnv('VITE_AZURE_CLIENT_ID')
  const tenantId = readEnv('VITE_AZURE_TENANT_ID')
  const apiScope = readEnv('VITE_API_SCOPE')
  const redirectUri = readEnv('VITE_REDIRECT_URI')

  const missing: string[] = []
  if (!clientId) missing.push('VITE_AZURE_CLIENT_ID')
  if (!tenantId) missing.push('VITE_AZURE_TENANT_ID')
  if (!apiScope) missing.push('VITE_API_SCOPE')
  if (!redirectUri) missing.push('VITE_REDIRECT_URI')

  if (missing.length > 0) throw new MsalConfigError(missing)

  return { clientId, tenantId, apiScope, redirectUri }
}

export interface ConsumerEntraEnvConfig {
  clientId: string
  authority: string
  apiScope: string
  redirectUri: string
}

/**
 * Entra External ID (CIAM) consumer/policyholder tenant config -- work order
 * Phase 8. Unlike `getEntraConfig()`, missing config here is NOT a startup
 * error: consumer sign-in is an optional capability. When any variable is
 * absent, `signInAsConsumer()` (AuthProvider) is simply unavailable rather
 * than the whole app failing closed -- staff auth is what's mandatory here.
 */
export function getConsumerEntraConfig(): ConsumerEntraEnvConfig | null {
  const clientId = readEnv('VITE_AZURE_CONSUMER_CLIENT_ID')
  const authority = readEnv('VITE_AZURE_CONSUMER_AUTHORITY')
  const apiScope = readEnv('VITE_AZURE_CONSUMER_API_SCOPE')
  const redirectUri = readEnv('VITE_CONSUMER_REDIRECT_URI')

  if (!clientId || !authority || !apiScope || !redirectUri) return null
  return { clientId, authority, apiScope, redirectUri }
}

/**
 * Optional Entra External ID social identity providers, configured entirely
 * server-side as authorities/policies — never a client-only `domain_hint`
 * trick. Format: JSON array of `{ "label": string, "authority": string }`.
 * Absent or invalid -> no social buttons are rendered.
 */
export interface SocialProvider {
  label: string
  authority: string
}

export function getSocialProviders(): SocialProvider[] {
  const raw = readEnv('VITE_SOCIAL_AUTHORITIES')
  if (!raw) return []
  try {
    const parsed = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed.filter(
      (p): p is SocialProvider =>
        p && typeof p.label === 'string' && typeof p.authority === 'string' && p.authority.startsWith('https://'),
    )
  } catch {
    return []
  }
}
