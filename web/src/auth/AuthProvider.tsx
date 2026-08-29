import { createContext, useContext, useEffect, useState, type ReactNode } from 'react'
import { PublicClientApplication, type AccountInfo, type Configuration } from '@azure/msal-browser'
import { MsalProvider, useMsal } from '@azure/msal-react'
import {
  getEntraConfig,
  getConsumerEntraConfig,
  getSocialProviders,
  MsalConfigError,
  type SocialProvider,
  type ConsumerEntraEnvConfig,
} from './config'
import { mapClaimsToRoles, type PasRole } from './roles'
import { setTokenProvider, setCurrentUserId } from '../api/client'
import ConfigErrorScreen from './ConfigErrorScreen'

export type UserKind = 'staff' | 'consumer' | null

interface AuthContextValue {
  isAuthenticated: boolean
  isLoading: boolean
  account: AccountInfo | null
  userKind: UserKind
  roles: PasRole[]
  socialProviders: SocialProvider[]
  consumerAvailable: boolean
  signIn: () => Promise<void>
  signInWithProvider: (authority: string) => Promise<void>
  signInAsConsumer: () => Promise<void>
  signOut: () => Promise<void>
}

const AuthContext = createContext<AuthContextValue | null>(null)

export function useAuth(): AuthContextValue {
  const ctx = useContext(AuthContext)
  if (!ctx) throw new Error('useAuth must be used within AuthProvider')
  return ctx
}

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

function AuthProviderInner({
  children,
  apiScope,
  consumerInstance,
  consumerCfg,
}: {
  children: ReactNode
  apiScope: string
  consumerInstance: PublicClientApplication | null
  consumerCfg: ConsumerEntraEnvConfig | null
}) {
  const { instance: staffInstance, accounts: staffAccounts, inProgress } = useMsal()
  const [consumerAccounts, setConsumerAccounts] = useState<AccountInfo[]>([])
  const [consumerReady, setConsumerReady] = useState(!consumerInstance)
  const [roles, setRoles] = useState<PasRole[]>([])
  const [rolesReady, setRolesReady] = useState(false)
  const [tokenProviderReady, setTokenProviderReady] = useState(false)

  const isStaff = staffAccounts.length > 0
  const isConsumer = !isStaff && consumerAccounts.length > 0
  const account = staffAccounts[0] ?? consumerAccounts[0] ?? null
  const userKind: UserKind = isStaff ? 'staff' : isConsumer ? 'consumer' : null

  // Consumer instance initializes independently of the staff one that
  // MsalProvider already owns -- it needs its own init/redirect handling.
  useEffect(() => {
    if (!consumerInstance) return
    let cancelled = false
    consumerInstance
      .initialize()
      .then(() => consumerInstance.handleRedirectPromise())
      .catch(() => undefined)
      .finally(() => {
        if (cancelled) return
        setConsumerAccounts(consumerInstance.getAllAccounts())
        setConsumerReady(true)
      })
    return () => { cancelled = true }
  }, [consumerInstance])

  useEffect(() => {
    setRolesReady(false)
    if (isStaff) {
      staffInstance
        .acquireTokenSilent({ scopes: [apiScope], account: staffAccounts[0] })
        .then(result => setRoles(rolesFromAccessToken(result.accessToken)))
        .catch(() => setRoles([]))
        .finally(() => setRolesReady(true))
    } else if (isConsumer && consumerInstance && consumerCfg) {
      consumerInstance
        .acquireTokenSilent({ scopes: [consumerCfg.apiScope], account: consumerAccounts[0] })
        .then(result => setRoles(rolesFromAccessToken(result.accessToken)))
        .catch(() => setRoles([]))
        .finally(() => setRolesReady(true))
    } else {
      setRoles([])
      setRolesReady(true)
    }
  }, [staffInstance, consumerInstance, isStaff, isConsumer, staffAccounts, consumerAccounts, apiScope, consumerCfg])

  useEffect(() => {
    setCurrentUserId(account?.localAccountId ?? null)
  }, [account])

  useEffect(() => {
    setTokenProviderReady(false)
    if (isStaff) {
      const staffAccount = staffAccounts[0]
      setTokenProvider(async () => {
        try {
          const result = await staffInstance.acquireTokenSilent({ scopes: [apiScope], account: staffAccount })
          setRoles(rolesFromAccessToken(result.accessToken))
          return result.accessToken
        } catch {
          await staffInstance.acquireTokenRedirect({ scopes: [apiScope], account: staffAccount })
          return null
        }
      })
    } else if (isConsumer && consumerInstance && consumerCfg) {
      const consumerAccount = consumerAccounts[0]
      setTokenProvider(async () => {
        try {
          const result = await consumerInstance.acquireTokenSilent({
            scopes: [consumerCfg.apiScope],
            account: consumerAccount,
          })
          setRoles(rolesFromAccessToken(result.accessToken))
          return result.accessToken
        } catch {
          await consumerInstance.acquireTokenRedirect({ scopes: [consumerCfg.apiScope], account: consumerAccount })
          return null
        }
      })
    } else {
      setTokenProvider(null)
    }
    setTokenProviderReady(true)
    return () => {
      setTokenProvider(null)
      setTokenProviderReady(false)
    }
  }, [staffInstance, consumerInstance, isStaff, isConsumer, staffAccounts, consumerAccounts, apiScope, consumerCfg])

  const signIn = () => staffInstance.loginRedirect({ scopes: ['openid', 'profile', apiScope] })
  const signInWithProvider = (authority: string) =>
    staffInstance.loginRedirect({ scopes: ['openid', 'profile', apiScope], authority })
  const signInAsConsumer = async () => {
    if (!consumerInstance || !consumerCfg) return
    await consumerInstance.loginRedirect({ scopes: [consumerCfg.apiScope] })
  }
  const signOut = () =>
    isConsumer && consumerInstance
      ? consumerInstance.logoutRedirect()
      : staffInstance.logoutRedirect()

  return (
    <AuthContext.Provider
      value={{
        isAuthenticated: isStaff || isConsumer,
        // Do not mount API-consuming screens until their bearer-token provider
        // is installed. Without this gate, child effects race the parent effect
        // after an Entra redirect and send unauthenticated first requests.
        isLoading:
          inProgress !== 'none' ||
          !consumerReady ||
          ((isStaff || isConsumer) && (!tokenProviderReady || !rolesReady)),
        account,
        userKind,
        roles,
        socialProviders: getSocialProviders(),
        consumerAvailable: !!consumerInstance,
        signIn,
        signInWithProvider,
        signInAsConsumer,
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
  const [consumerInstance, setConsumerInstance] = useState<PublicClientApplication | null>(null)
  const [apiScope, setApiScope] = useState('')
  const [consumerCfg, setConsumerCfg] = useState<ConsumerEntraEnvConfig | null>(null)

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

      // Consumer/CIAM tenant (work order Phase 8) -- optional. A second,
      // independent PublicClientApplication: it's a genuinely separate app
      // registration in a separate tenant, not reachable via the staff
      // instance's signInWithProvider (that only federates providers within
      // the SAME tenant/app registration).
      const cCfg = getConsumerEntraConfig()
      let cInstance: PublicClientApplication | null = null
      if (cCfg) {
        const consumerConfig: Configuration = {
          auth: {
            clientId: cCfg.clientId,
            authority: cCfg.authority,
            redirectUri: cCfg.redirectUri,
            postLogoutRedirectUri: cCfg.redirectUri,
          },
          cache: {
            cacheLocation: 'sessionStorage',
            storeAuthStateInCookie: false,
          },
        }
        cInstance = new PublicClientApplication(consumerConfig)
      }

      setInstance(pca)
      setConsumerInstance(cInstance)
      setConsumerCfg(cCfg)
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
      <AuthProviderInner apiScope={apiScope} consumerInstance={consumerInstance} consumerCfg={consumerCfg}>
        {children}
      </AuthProviderInner>
    </MsalProvider>
  )
}
