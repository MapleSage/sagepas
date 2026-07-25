import { createContext, useContext, useEffect, useState, type ReactNode } from 'react'
import { PublicClientApplication, type AccountInfo } from '@azure/msal-browser'
import { MsalProvider, useMsal } from '@azure/msal-react'
import { getEntraConfig, getSocialProviders, MsalConfigError, type SocialProvider } from './config'
import { mapClaimsToRoles, type PasRole } from './roles'
import { setTokenProvider, setCurrentUserId } from '../api/client'
import ConfigErrorScreen from './ConfigErrorScreen'

interface AuthContextValue {
  isAuthenticated: boolean
  isLoading: boolean
  account: AccountInfo | null
  roles: PasRole[]
  socialProviders: SocialProvider[]
  signIn: () => Promise<void>
  signInWithProvider: (authority: string) => Promise<void>
  signOut: () => Promise<void>
}

const AuthContext = createContext<AuthContextValue | null>(null)

export function useAuth(): AuthContextValue {
  const ctx = useContext(AuthContext)
  if (!ctx) throw new Error('useAuth must be used within AuthProvider')
  return ctx
}

function AuthProviderInner({ children, apiScope }: { children: ReactNode; apiScope: string }) {
  const { instance, accounts, inProgress } = useMsal()
  const account = accounts[0] ?? null
  const [roles, setRoles] = useState<PasRole[]>([])
  const [rolesReady, setRolesReady] = useState(false)
  const [tokenProviderReady, setTokenProviderReady] = useState(false)

  function rolesFromAccessToken(token: string): PasRole[] {
    try {
      const encoded = token.split('.')[1]
      const normalized = encoded.replace(/-/g, '+').replace(/_/g, '/')
      const claims = JSON.parse(window.atob(normalized)) as Record<string, unknown>
      const roleClaims = Array.isArray(claims.roles) ? claims.roles : []
      const groupClaims = Array.isArray(claims.groups) ? claims.groups : []
      return mapClaimsToRoles([...roleClaims, ...groupClaims])
    } catch {
      return []
    }
  }

  useEffect(() => {
    setRolesReady(false)
    if (!account) {
      setRoles([])
      setRolesReady(true)
      return
    }
    instance.acquireTokenSilent({ scopes: [apiScope], account })
      .then(result => setRoles(rolesFromAccessToken(result.accessToken)))
      .catch(() => setRoles([]))
      .finally(() => setRolesReady(true))
  }, [instance, account, apiScope])

  useEffect(() => {
    setCurrentUserId(account?.localAccountId ?? null)
  }, [account])

  useEffect(() => {
    setTokenProviderReady(false)
    setTokenProvider(
      account
        ? async () => {
            try {
              const result = await instance.acquireTokenSilent({ scopes: [apiScope], account })
              setRoles(rolesFromAccessToken(result.accessToken))
              return result.accessToken
            } catch {
              // Silent acquisition failed (expired session, revoked consent, etc).
              // Redirect through Entra again rather than surfacing a raw 401.
              await instance.acquireTokenRedirect({ scopes: [apiScope], account })
              return null
            }
          }
        : null,
    )
    setTokenProviderReady(true)
    return () => {
      setTokenProvider(null)
      setTokenProviderReady(false)
    }
  }, [instance, account, apiScope])

  const signIn = () => instance.loginRedirect({ scopes: ['openid', 'profile', apiScope] })
  const signInWithProvider = (authority: string) =>
    instance.loginRedirect({ scopes: ['openid', 'profile', apiScope], authority })
  const signOut = () => instance.logoutRedirect()

  return (
    <AuthContext.Provider
      value={{
        isAuthenticated: accounts.length > 0,
        // Do not mount API-consuming screens until their bearer-token provider
        // is installed. Without this gate, child effects race the parent effect
        // after an Entra redirect and send unauthenticated first requests.
        isLoading: inProgress !== 'none' || (!!account && (!tokenProviderReady || !rolesReady)),
        account,
        roles,
        socialProviders: getSocialProviders(),
        signIn,
        signInWithProvider,
        signOut,
      }}
    >
      {children}
    </AuthContext.Provider>
  )
}

export function AuthProvider({ children }: { children: ReactNode }) {
  const [configError, setConfigError] = useState<MsalConfigError | null>(null)
  const [ready, setReady] = useState(false)
  const [instance, setInstance] = useState<PublicClientApplication | null>(null)
  const [apiScope, setApiScope] = useState('')

  useEffect(() => {
    let cancelled = false
    async function init() {
      let cfg
      try {
        cfg = getEntraConfig()
      } catch (e) {
        if (e instanceof MsalConfigError) { if (!cancelled) setConfigError(e); return }
        throw e
      }
      const pca = new PublicClientApplication({
        auth: {
          clientId: cfg.clientId,
          authority: `https://login.microsoftonline.com/${cfg.tenantId}`,
          redirectUri: cfg.redirectUri,
          postLogoutRedirectUri: cfg.redirectUri,
        },
        cache: {
          cacheLocation: 'sessionStorage',
          storeAuthStateInCookie: false,
        },
      })
      await pca.initialize()
      await pca.handleRedirectPromise().catch(() => undefined)
      if (cancelled) return
      setInstance(pca)
      setApiScope(cfg.apiScope)
      setReady(true)
    }
    init()
    return () => { cancelled = true }
  }, [])

  if (configError) return <ConfigErrorScreen missing={configError.missing} />
  if (!ready || !instance) return null

  return (
    <MsalProvider instance={instance}>
      <AuthProviderInner apiScope={apiScope}>{children}</AuthProviderInner>
    </MsalProvider>
  )
}
