import { D } from '../theme'
import { ROLE_LABELS, type PasRole } from './roles'
import { useAuth } from './AuthProvider'

export default function AccessDenied({ allow }: { allow: PasRole[] }) {
  const { account, roles, signOut } = useAuth()
  return (
    <div style={{ minHeight: '100vh', display: 'grid', placeItems: 'center', background: D.bg, padding: 24 }}>
      <div style={{ width: 520, maxWidth: '100%', background: '#fff', borderRadius: 14, border: `1px solid ${D.border}`, boxShadow: '0 18px 45px rgba(13,43,61,.12)', padding: 32, textAlign: 'center' }}>
        <div style={{ fontSize: 12, letterSpacing: '.14em', color: D.orange, fontWeight: 700, marginBottom: 10 }}>ACCESS DENIED</div>
        <h1 style={{ fontSize: 22, color: D.text, margin: '0 0 12px' }}>No permitted role</h1>
        <p style={{ fontSize: 13, color: D.textSub, lineHeight: 1.6, margin: '0 0 8px' }}>
          {account?.username || account?.name || 'Your account'} does not hold a PAS role required
          for this workspace ({allow.map(r => ROLE_LABELS[r]).join(', ')}).
        </p>
        <p style={{ fontSize: 12, color: D.textSub, margin: '0 0 20px' }}>
          Current roles: {roles.length ? roles.map(r => ROLE_LABELS[r]).join(', ') : 'none assigned'}.
          Ask an administrator to grant a PAS app role in Microsoft Entra ID.
        </p>
        <button onClick={() => signOut()} style={{ padding: '10px 20px', border: 0, borderRadius: 8, background: D.navy, color: '#fff', fontWeight: 700, cursor: 'pointer' }}>
          Sign out
        </button>
      </div>
    </div>
  )
}
