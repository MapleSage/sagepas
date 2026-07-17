import { D } from '../theme'

export default function ConfigErrorScreen({ missing }: { missing: string[] }) {
  return (
    <div style={{ minHeight: '100vh', display: 'grid', placeItems: 'center', background: D.bg, padding: 24 }}>
      <div style={{ width: 520, maxWidth: '100%', background: '#fff', borderRadius: 14, border: `1px solid ${D.border}`, boxShadow: '0 18px 45px rgba(13,43,61,.12)', padding: 32 }}>
        <div style={{ fontSize: 12, letterSpacing: '.14em', color: D.red, fontWeight: 700, marginBottom: 10 }}>AUTHENTICATION NOT CONFIGURED</div>
        <h1 style={{ fontSize: 22, color: D.text, margin: '0 0 12px' }}>Sign-in is unavailable</h1>
        <p style={{ fontSize: 13, color: D.textSub, lineHeight: 1.6, margin: '0 0 16px' }}>
          This deployment is missing required Microsoft Entra ID configuration. The app refuses to
          fall back to a default tenant or client — set the following environment variables and rebuild:
        </p>
        <ul style={{ margin: 0, padding: '0 0 0 18px', fontSize: 13, color: D.text, lineHeight: 1.9 }}>
          {missing.map(name => (
            <li key={name}><code style={{ background: D.bg, padding: '2px 6px', borderRadius: 4 }}>{name}</code></li>
          ))}
        </ul>
      </div>
    </div>
  )
}
