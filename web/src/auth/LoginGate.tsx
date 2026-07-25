import { useState, type ReactNode } from 'react'
import { D } from '../theme'
import { useAuth } from './AuthProvider'

function MicrosoftLogo() {
  return (
    <svg width="18" height="18" viewBox="0 0 21 21" aria-hidden="true">
      <rect x="1" y="1" width="9" height="9" fill="#F25022" />
      <rect x="11" y="1" width="9" height="9" fill="#7FBA00" />
      <rect x="1" y="11" width="9" height="9" fill="#00A4EF" />
      <rect x="11" y="11" width="9" height="9" fill="#FFB900" />
    </svg>
  )
}

export default function LoginGate({ children }: { children: ReactNode }) {
  const { isAuthenticated, isLoading, socialProviders, signIn, signInWithProvider } = useAuth()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  if (isAuthenticated && !isLoading) return <>{children}</>
  if (isAuthenticated) {
    return (
      <div style={{ minHeight: '100vh', display: 'grid', placeItems: 'center', background: D.bg, color: D.text }}>
        <div style={{ fontSize: 13, fontWeight: 600 }}>Preparing your SagePAS workspace…</div>
      </div>
    )
  }

  async function handleSignIn(authority?: string) {
    setBusy(true)
    setError('')
    try {
      await (authority ? signInWithProvider(authority) : signIn())
    } catch (e: any) {
      setError(e?.message || 'Sign-in failed to start. Please try again.')
      setBusy(false)
    }
  }

  return (
    <div style={{ minHeight: '100vh', display: 'grid', gridTemplateColumns: '1.1fr .9fr', background: D.bg }}>
      <div style={{ background: D.navy, color: '#fff', padding: '9vh 7vw', display: 'flex', flexDirection: 'column', justifyContent: 'center' }}>
        <div style={{ fontSize: 13, letterSpacing: '.18em', color: D.teal, fontWeight: 700 }}>SAGESURE</div>
        <h1 style={{ fontSize: 42, lineHeight: 1.08, margin: '18px 0' }}>Policy Administration System</h1>
        <p style={{ color: D.navyText, maxWidth: 520, lineHeight: 1.7 }}>
          Native Rust quote, bind, issue, policy servicing, dealer and reporting workflows.
        </p>
      </div>
      <div style={{ display: 'grid', placeItems: 'center', padding: 30 }}>
        <div style={{ width: 380, maxWidth: '100%', background: '#fff', padding: 28, borderRadius: 14, border: `1px solid ${D.border}`, boxShadow: '0 18px 45px rgba(13,43,61,.12)' }}>
          <h2 style={{ margin: '0 0 5px', color: D.text }}>Sign in</h2>
          <p style={{ margin: '0 0 20px', fontSize: 12, color: D.textSub }}>
            Authentication is handled entirely by your organization's identity provider.
          </p>

          {error && <div style={{ padding: 10, background: '#FFF1F1', color: '#9B2C2C', borderRadius: 7, fontSize: 12, marginBottom: 12 }}>{error}</div>}

          <button
            onClick={() => handleSignIn()}
            disabled={busy || isLoading}
            style={{
              width: '100%', display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 10,
              padding: '11px 0', border: `1px solid ${D.border}`, borderRadius: 8, background: '#fff',
              color: D.text, fontWeight: 600, fontSize: 13, cursor: busy || isLoading ? 'default' : 'pointer',
            }}
          >
            <MicrosoftLogo />
            {busy || isLoading ? 'Redirecting…' : 'Sign in with Microsoft'}
          </button>

          {socialProviders.length > 0 && (
            <div style={{ display: 'grid', gap: 8, marginTop: 10 }}>
              {socialProviders.map(provider => (
                <button
                  key={provider.authority}
                  onClick={() => handleSignIn(provider.authority)}
                  disabled={busy || isLoading}
                  style={{ width: '100%', padding: '10px 0', border: `1px solid ${D.border}`, borderRadius: 8, background: '#fff', color: D.text, fontWeight: 600, fontSize: 12, cursor: busy || isLoading ? 'default' : 'pointer' }}
                >
                  Sign in with {provider.label}
                </button>
              ))}
            </div>
          )}

          <div style={{ marginTop: 18, paddingTop: 16, borderTop: `1px solid ${D.border}`, fontSize: 11, color: D.textSub, lineHeight: 1.6 }}>
            When enabled by your organization, sign-in supports Face ID, Touch ID, Windows Hello,
            and security keys — presented and verified entirely by Microsoft Entra ID during
            sign-in. This app never captures camera or biometric data itself.
          </div>
        </div>
      </div>
    </div>
  )
}
