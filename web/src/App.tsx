import { useState, useRef, useEffect } from 'react'
import { AreaChart, Area, LineChart, Line, XAxis, YAxis, CartesianGrid, Legend, Tooltip as RechartsTip, ResponsiveContainer } from 'recharts'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { BrowserRouter, Routes, Route, useNavigate, useLocation } from 'react-router-dom'
import { FluentProvider, createLightTheme, Tooltip } from '@fluentui/react-components'
import type { BrandVariants } from '@fluentui/react-components'
import {
  HomeRegular, HomeFilled,
  NavigationRegular,
  SearchRegular,
  AlertRegular,
  MoneyRegular, MoneyFilled,
  ShieldRegular, ShieldFilled,
  ChartMultipleRegular, ChartMultipleFilled,
  BuildingBankRegular, BuildingBankFilled,
} from '@fluentui/react-icons'
import QuotesPage from './components/quotes/QuotesPage'
import PoliciesPage from './components/policies/PoliciesPage'
import DealersPage from './components/dealers/DealersPage'
import ReportsPage from './components/reports/ReportsPage'
import { pasApi } from './api/client'
import { D } from './theme'
import { AuthProvider, useAuth } from './auth/AuthProvider'
import LoginGate from './auth/LoginGate'
import RoleGuard from './auth/RoleGuard'
import { ROLE_LABELS } from './auth/roles'

// ── Fluent UI brand — SageSure blue (light theme) ───────────────────────────
const brand: BrandVariants = {
  10: '#030A12', 20: '#061423', 30: '#0A2035', 40: '#0D2B3D',
  50: '#103750', 60: '#144264', 70: '#174D6D', 80: '#216884',
  90: '#2C839A', 100: '#3D9CA2', 110: '#60B2B7', 120: '#8FC8CB',
  130: '#BFDDE0', 140: '#DCECEF', 150: '#ECF5F8', 160: '#F7FBFD',
}
const sageSureTheme = createLightTheme(brand)

const PAS_APP_ROLES = ['admin', 'agent', 'underwriter', 'customer'] as const

const queryClient = new QueryClient({
  defaultOptions: { queries: { refetchOnWindowFocus: false, retry: 1, staleTime: 5 * 60 * 1000 } },
})

// ── Nav tabs — role-based ─────────────────────────────────────────────────────
type SubItem = { label: string; path: string }
type TabDef = {
  id: string; path: string; label: string
  icon: React.ComponentType<{ style?: React.CSSProperties }>
  iconFilled: React.ComponentType<{ style?: React.CSSProperties }>
  sub: SubItem[]
}

const ALL_TABS: TabDef[] = [
  { id: 'dashboard', path: '/', label: 'Home', icon: HomeRegular, iconFilled: HomeFilled,
    sub: [{ label: 'Overview', path: '/' }, { label: 'Activity Feed', path: '/' }] },
  { id: 'quotes', path: '/quotes', label: 'Quotes', icon: MoneyRegular, iconFilled: MoneyFilled,
    sub: [{ label: 'All Quotes', path: '/quotes' }, { label: 'Quick Quote', path: '/quotes?view=quick' }, { label: 'Full Quote', path: '/quotes?view=full' }] },
  { id: 'policies', path: '/policies', label: 'Policies', icon: ShieldRegular, iconFilled: ShieldFilled,
    sub: [{ label: 'All Policies', path: '/policies' }, { label: 'Active', path: '/policies?status=ISSUED' }, { label: 'Endorsed', path: '/policies?status=ENDORSED' }, { label: 'Cancelled', path: '/policies?status=CANCELLED' }] },
  { id: 'dealers', path: '/dealers', label: 'Dealers', icon: BuildingBankRegular, iconFilled: BuildingBankFilled,
    sub: [{ label: 'All Dealers', path: '/dealers' }, { label: 'Add Dealer', path: '/dealers?action=add' }, { label: 'Commissions', path: '/dealers' }] },
  { id: 'reports', path: '/reports', label: 'Reports', icon: ChartMultipleRegular, iconFilled: ChartMultipleFilled,
    sub: [{ label: 'Overview', path: '/reports' }, { label: 'BDX Export', path: '/reports?view=bdx' }] },
]

const SIDEBAR_W_EXP = 220
const SIDEBAR_W_COL = 56

// ── KPI card ──────────────────────────────────────────────────────────────────
function MetricCard({ label, value, delta, color, sparkData, onClick }: {
  label: string; value: string | number; delta?: string; color: string;
  sparkData?: { d: number; v: number }[]; onClick?: () => void
}) {
  const [hover, setHover] = useState(false)
  return (
    <button
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        background: D.surface, border: `1px solid ${hover ? color : D.border}`,
        borderTop: `3px solid ${color}`, borderRadius: 10, padding: '16px 18px 0',
        cursor: 'pointer', textAlign: 'left', width: '100%', overflow: 'hidden',
        boxShadow: hover ? '0 4px 16px rgba(0,0,0,0.1)' : '0 1px 4px rgba(0,0,0,0.05)',
        transition: 'all 150ms ease',
      }}
    >
      <div style={{ fontSize: 28, fontWeight: 700, color: D.text, lineHeight: 1, marginBottom: 2 }}>{value}</div>
      <div style={{ fontSize: 12, color: D.textSub, fontWeight: 500 }}>{label}</div>
      {delta && (
        <div style={{ fontSize: 11, marginTop: 4, fontWeight: 600, color: delta.startsWith('+') ? D.green : D.red }}>{delta}</div>
      )}
      {sparkData && (
        <div style={{ height: 48, marginTop: 8, marginLeft: -18, marginRight: -18 }}>
          <ResponsiveContainer width="100%" height="100%">
            <AreaChart data={sparkData} margin={{ top: 0, right: 0, left: 0, bottom: 0 }}>
              <defs>
                <linearGradient id={`sg-${label.replace(/\s+/g, '-')}`} x1="0" y1="0" x2="0" y2="1">
                  <stop offset="5%"  stopColor={color} stopOpacity={0.4} />
                  <stop offset="95%" stopColor={color} stopOpacity={0.02} />
                </linearGradient>
              </defs>
              <Area type="monotone" dataKey="v" stroke={color} strokeWidth={2}
                fill={`url(#sg-${label.replace(/\s+/g, '-')})`} dot={false} isAnimationActive={false} />
            </AreaChart>
          </ResponsiveContainer>
        </div>
      )}
    </button>
  )
}

// ── Status badge ──────────────────────────────────────────────────────────────
function StatusBadge({ status }: { status: 'up' | 'down' | 'checking' }) {
  const color = status === 'up' ? D.green : status === 'down' ? D.red : D.textSub
  return (
    <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
      <span style={{ width: 8, height: 8, borderRadius: '50%', background: color, flexShrink: 0, boxShadow: status === 'up' ? `0 0 0 3px ${D.green}33` : undefined }} />
      <span style={{ fontSize: 11, color, fontWeight: 500 }}>
        {status === 'up' ? 'Operational' : status === 'down' ? 'Unavailable' : 'Checking…'}
      </span>
    </span>
  )
}

function ServiceCard({ label, url, okStatuses = [200] }: { label: string; url: string; okStatuses?: number[] }) {
  const [status, setStatus] = useState<'checking' | 'up' | 'down'>('checking')
  useEffect(() => {
    let cancelled = false
    const ac = new AbortController()
    const t = setTimeout(() => ac.abort(), 6000)
    fetch(url, { signal: ac.signal })
      .then(r => { if (!cancelled) setStatus((r.ok || okStatuses.includes(r.status)) ? 'up' : 'down') })
      .catch(() => { if (!cancelled) setStatus('down') })
      .finally(() => clearTimeout(t))
    return () => { cancelled = true; clearTimeout(t); ac.abort() }
  }, [url, okStatuses])
  return (
    <div style={{ background: D.surface, border: `1px solid ${D.border}`, borderRadius: 10, padding: '14px 18px', display: 'flex', alignItems: 'center', justifyContent: 'space-between', boxShadow: '0 1px 3px rgba(0,0,0,0.04)' }}>
      <span style={{ fontSize: 13, fontWeight: 600, color: D.text }}>{label}</span>
      <StatusBadge status={status} />
    </div>
  )
}

function ActionCard({ icon, title, desc, onClick, accent }: { icon: string; title: string; desc: string; onClick: () => void; accent: string }) {
  const [hover, setHover] = useState(false)
  return (
    <button
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        background: hover ? '#FAFBFD' : D.surface, border: `1px solid ${D.border}`,
        borderLeft: `4px solid ${accent}`, borderRadius: 10, padding: '16px 18px',
        textAlign: 'left', cursor: 'pointer', width: '100%',
        boxShadow: hover ? '0 4px 12px rgba(0,0,0,0.08)' : '0 1px 3px rgba(0,0,0,0.04)',
        transition: 'all 150ms ease',
      }}
    >
      <span style={{ fontSize: 20, display: 'block', marginBottom: 8 }}>{icon}</span>
      <span style={{ fontSize: 13, fontWeight: 600, color: D.text, display: 'block', marginBottom: 3 }}>{title}</span>
      <span style={{ fontSize: 11, color: D.textSub }}>{desc}</span>
    </button>
  )
}

// ── Dashboard ─────────────────────────────────────────────────────────────────
function DashboardPage({ onNavigate }: { onNavigate: (path: string) => void }) {
  const [metrics, setMetrics] = useState({ quotes: 0, quoted: 0, bound: 0, policies: 0 })
  const [available, setAvailable] = useState(true)

  useEffect(() => {
    let cancelled = false
    Promise.all([
      pasApi.quotes(),
      pasApi.policies(),
    ]).then(([quoteBody, policyBody]) => {
      if (cancelled) return
      const quotes = Array.isArray(quoteBody) ? quoteBody : (quoteBody?.items || quoteBody?.quotes || [])
      const policies = Array.isArray(policyBody) ? policyBody : (policyBody?.items || policyBody?.policies || [])
      setMetrics({
        quotes: quotes.length,
        quoted: quotes.filter((q: any) => ['QUICK_QUOTE', 'QUOTED'].includes(String(q.state || q.status || '').toUpperCase())).length,
        bound: quotes.filter((q: any) => String(q.state || q.status || '').toUpperCase() === 'BOUND').length,
        policies: policies.length,
      })
      setAvailable(true)
    }).catch(() => { if (!cancelled) setAvailable(false) })
    return () => { cancelled = true }
  }, [])

  const trend = [
    { day: 'Mon', quotes: 0, policies: 0 }, { day: 'Tue', quotes: 0, policies: 0 },
    { day: 'Wed', quotes: 0, policies: 0 }, { day: 'Thu', quotes: 0, policies: 0 },
    { day: 'Fri', quotes: 0, policies: 0 }, { day: 'Sat', quotes: 0, policies: 0 },
    { day: 'Sun', quotes: metrics.quotes, policies: metrics.policies },
  ]

  return (
    <div style={{ padding: '24px 28px', maxWidth: 1440, margin: '0 auto' }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: 24 }}>
        <div>
          <h1 style={{ fontSize: 24, color: D.text, margin: 0 }}>Policy Administration</h1>
          <p style={{ fontSize: 12, color: D.textSub, marginTop: 5 }}>Quotes, policies, dealers, commissions, documents, and reporting</p>
        </div>
        <button onClick={() => onNavigate('/quotes?view=full')} style={{ background: D.orange, color: '#fff', border: 0, borderRadius: 8, padding: '10px 18px', fontSize: 13, fontWeight: 700, cursor: 'pointer' }}>+ New Quote</button>
      </div>

      {!available && <div style={{ background: '#FFF6ED', border: '1px solid #FFC391', borderLeft: `4px solid ${D.orange}`, color: '#98440E', borderRadius: 8, padding: '11px 14px', fontSize: 12, marginBottom: 18 }}>Native PAS API is unavailable. No sample data is being substituted.</div>}

      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4,minmax(0,1fr))', gap: 14, marginBottom: 22 }}>
        <MetricCard label="All Quotes" value={metrics.quotes} color={D.teal} onClick={() => onNavigate('/quotes')} />
        <MetricCard label="Quoted" value={metrics.quoted} color={D.purple} onClick={() => onNavigate('/quotes')} />
        <MetricCard label="Bound Quotes" value={metrics.bound} color={D.orange} onClick={() => onNavigate('/quotes')} />
        <MetricCard label="Policies" value={metrics.policies} color={D.green} onClick={() => onNavigate('/policies')} />
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: 'minmax(0,2fr) minmax(280px,1fr)', gap: 16, marginBottom: 22 }}>
        <section style={{ background: D.surface, border: `1px solid ${D.border}`, borderRadius: 10, padding: '18px 20px', minHeight: 280 }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 18 }}><div><strong style={{ color: D.text, fontSize: 14 }}>PAS activity</strong><div style={{ color: D.textSub, fontSize: 11, marginTop: 3 }}>Live native records only</div></div><button onClick={() => onNavigate('/reports')} style={{ border: 0, background: 'transparent', color: D.teal, fontWeight: 600, cursor: 'pointer' }}>View reports →</button></div>
          <div style={{ height: 210 }}><ResponsiveContainer width="100%" height="100%"><LineChart data={trend}><CartesianGrid stroke={D.border} strokeDasharray="3 3" vertical={false}/><XAxis dataKey="day" tick={{fontSize:10,fill:D.textSub}}/><YAxis allowDecimals={false} tick={{fontSize:10,fill:D.textSub}}/><RechartsTip/><Legend/><Line dataKey="quotes" name="Quotes" stroke={D.teal} strokeWidth={2.2}/><Line dataKey="policies" name="Policies" stroke={D.orange} strokeWidth={2.2}/></LineChart></ResponsiveContainer></div>
        </section>
        <section style={{ background: D.surface, border: `1px solid ${D.border}`, borderRadius: 10, padding: '18px 20px' }}>
          <strong style={{ color: D.text, fontSize: 14 }}>Quick actions</strong>
          <div style={{ display: 'grid', gap: 10, marginTop: 15 }}>
            <ActionCard icon="＋" title="Quick Quote" desc="Create an indicative quote" accent={D.orange} onClick={() => onNavigate('/quotes?view=quick')} />
            <ActionCard icon="▤" title="Policy register" desc="Search and manage policies" accent={D.teal} onClick={() => onNavigate('/policies')} />
            <ActionCard icon="⌂" title="Dealer management" desc="Dealer and commission setup" accent={D.purple} onClick={() => onNavigate('/dealers')} />
          </div>
        </section>
      </div>
    </div>
  )
}

// ── Profile dropdown ──────────────────────────────────────────────────────────
function ProfileDropdown() {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [open])

  const { account, roles, signOut } = useAuth()
  const name = account?.name || account?.username || 'SageSure User'
  const email = account?.username || ''
  const initials = name.split(' ').map((n: string) => n[0]).slice(0, 2).join('').toUpperCase()
  const roleSummary = roles.length ? roles.map(r => ROLE_LABELS[r]).join(', ') : 'No PAS role assigned'

  const links = [
    { label: 'SageSure Website', url: 'https://sagesure.io/' },
    { label: 'Privacy Policy',   url: 'https://sagesure.io/privacy-policy' },
    { label: 'Terms of Use',     url: 'https://sagesure.io/term-of-use' },
    { label: 'Data Policy',      url: 'https://sagesure.io/data-policy' },
  ]

  return (
    <div ref={ref} style={{ position: 'relative' }}>
      <button
        onClick={() => setOpen(o => !o)}
        style={{ display: 'flex', alignItems: 'center', gap: 8, background: open ? 'rgba(255,255,255,0.12)' : 'rgba(255,255,255,0.07)', border: `1px solid ${open ? D.teal : D.navyBorder}`, borderRadius: 8, padding: '5px 10px', cursor: 'pointer', color: '#fff', transition: 'all 120ms' }}
      >
        <div style={{ width: 28, height: 28, borderRadius: '50%', background: D.orange, display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 11, fontWeight: 700, color: '#fff', flexShrink: 0 }}>
          {initials}
        </div>
        <div style={{ textAlign: 'left' }}>
          <div style={{ fontSize: 12, fontWeight: 600, lineHeight: 1, color: '#fff' }}>{name}</div>
          <div style={{ fontSize: 10, color: D.navyText, marginTop: 2 }}>{roleSummary}</div>
        </div>
        <span style={{ fontSize: 9, color: D.navyMuted, marginLeft: 2 }}>▾</span>
      </button>

      {open && (
        <div style={{ position: 'absolute', top: 'calc(100% + 6px)', right: 0, background: '#fff', border: `1px solid ${D.border}`, borderRadius: 12, boxShadow: '0 8px 32px rgba(0,0,0,0.18)', minWidth: 252, zIndex: 9999, overflow: 'hidden' }}>
          <div style={{ padding: '14px 16px', borderBottom: `1px solid ${D.border}`, display: 'flex', gap: 10, alignItems: 'center' }}>
            <div style={{ width: 38, height: 38, borderRadius: '50%', background: D.orange, display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 14, fontWeight: 700, color: '#fff', flexShrink: 0 }}>
              {initials}
            </div>
            <div>
              <div style={{ fontSize: 13, fontWeight: 700, color: D.text }}>{name}</div>
              {email && <div style={{ fontSize: 11, color: D.textSub, marginTop: 2 }}>{email}</div>}
            </div>
          </div>
          <div style={{ padding: '6px 0', borderBottom: `1px solid ${D.border}` }}>
            {links.map(l => (
              <a
                key={l.url}
                href={l.url}
                target="_blank"
                rel="noopener noreferrer"
                onClick={() => setOpen(false)}
                style={{ display: 'block', padding: '9px 16px', fontSize: 12, color: D.text, textDecoration: 'none', transition: 'background 100ms' }}
                onMouseEnter={e => (e.currentTarget.style.background = D.bg)}
                onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
              >{l.label}</a>
            ))}
          </div>
          <div style={{ padding: '6px 0' }}>
            <button
              onClick={() => { setOpen(false); signOut() }}
              style={{ display: 'block', width: '100%', textAlign: 'left', padding: '9px 16px', fontSize: 12, fontWeight: 600, color: D.red, background: 'transparent', border: 'none', cursor: 'pointer', transition: 'background 100ms' }}
              onMouseEnter={e => (e.currentTarget.style.background = D.bg)}
              onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
            >Sign out</button>
          </div>
        </div>
      )}
    </div>
  )
}

// ── Sidebar nav item ───────────────────────────────────────────────────────────
function NavItem({ tab, isActive, label, Icon, expanded, onNavigate }: {
  tab: TabDef; isActive: boolean; label: string;
  Icon: React.ComponentType<{ style?: React.CSSProperties }>;
  expanded: boolean; onNavigate: (path: string) => void;
}) {
  const [hovered, setHovered] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  const [flyoutTop, setFlyoutTop] = useState(0)

  return (
    <div
      ref={ref}
      style={{ position: 'relative' }}
      onMouseEnter={() => { setHovered(true); if (ref.current) setFlyoutTop(ref.current.getBoundingClientRect().top) }}
      onMouseLeave={() => setHovered(false)}
    >
      <Tooltip content={!expanded ? label : ''} relationship="label" positioning="after">
        <button
          onClick={() => onNavigate(tab.path)}
          style={{
            display: 'flex', alignItems: 'center',
            justifyContent: expanded ? 'flex-start' : 'center',
            width: '100%', padding: expanded ? '10px 16px' : '10px 0',
            border: 'none',
            borderLeft: isActive ? `3px solid ${D.teal}` : '3px solid transparent',
            background: isActive ? 'rgba(14,116,144,0.18)' : hovered ? D.navyMid : 'transparent',
            color: isActive ? '#67E8F9' : D.navyText,
            cursor: 'pointer', transition: 'all 120ms ease',
            whiteSpace: 'nowrap', overflow: 'hidden', flexShrink: 0, textAlign: 'left',
          }}
        >
          <Icon style={{ fontSize: 17, flexShrink: 0, marginRight: expanded ? 10 : 0 }} />
          {expanded && (
            <span style={{ fontSize: 12, fontWeight: isActive ? 600 : 400, overflow: 'hidden', textOverflow: 'ellipsis', flex: 1 }}>
              {label}
            </span>
          )}
        </button>
      </Tooltip>

      {hovered && (
        <div style={{ position: 'fixed', left: expanded ? SIDEBAR_W_EXP : SIDEBAR_W_COL, top: flyoutTop, background: '#0F2744', border: `1px solid ${D.navyBorder}`, borderRadius: 10, boxShadow: '0 8px 32px rgba(0,0,0,0.4)', minWidth: 180, zIndex: 9999, overflow: 'hidden' }}>
          <div style={{ padding: '10px 14px 6px', borderBottom: `1px solid ${D.navyBorder}` }}>
            <div style={{ fontSize: 10, fontWeight: 700, color: D.teal, letterSpacing: '0.08em', textTransform: 'uppercase' }}>{label}</div>
          </div>
          {tab.sub.map(s => (
            <button
              key={s.label}
              onClick={() => { onNavigate(s.path); setHovered(false) }}
              style={{ display: 'block', width: '100%', textAlign: 'left', padding: '9px 14px', border: 'none', background: 'transparent', fontSize: 12, color: '#CBD5E1', cursor: 'pointer', transition: 'background 100ms, color 100ms' }}
              onMouseEnter={e => { e.currentTarget.style.background = D.navyMid; e.currentTarget.style.color = '#fff' }}
              onMouseLeave={e => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = '#CBD5E1' }}
            >{s.label}</button>
          ))}
        </div>
      )}
    </div>
  )
}

// ── App shell ─────────────────────────────────────────────────────────────────
function AppShell() {
  const [expanded, setExpanded] = useState(true)
  const navigate = useNavigate()
  const location = useLocation()
  const tabs = ALL_TABS

  const activeId = tabs.find(t =>
    t.path === '/' ? location.pathname === '/' : location.pathname.startsWith(t.path)
  )?.id ?? tabs[0]?.id ?? 'dashboard'

  return (
    <div style={{ height: '100vh', display: 'flex', flexDirection: 'column', overflow: 'hidden', background: D.bg }}>

      {/* ── Header ── */}
      <header style={{ flexShrink: 0, height: 56, display: 'flex', alignItems: 'center', background: D.navy, borderBottom: `1px solid ${D.navyBorder}`, padding: '0 16px', gap: 16, zIndex: 100 }}>
        {/* Logo */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexShrink: 0 }}>
          <div style={{ background: '#ffffff', borderRadius: 10, width: 36, height: 36, display: 'flex', alignItems: 'center', justifyContent: 'center', boxShadow: '0 2px 8px rgba(0,0,0,0.25)' }}>
            <img src="/sagesure_logo.jpeg" alt="SageSure" style={{ height: 28, width: 28, objectFit: 'contain', borderRadius: 6 }} onError={e => { (e.currentTarget as HTMLImageElement).style.display = 'none' }} />
          </div>
          <div>
            <div style={{ fontWeight: 700, fontSize: 13, color: '#fff', lineHeight: 1 }}>SAGESURE</div>
            <div style={{ fontSize: 10, color: D.navyText, lineHeight: 1.3, marginTop: 2 }}>Insurance Workspace</div>
          </div>
        </div>

        {/* Search */}
        <div style={{ flex: 1, maxWidth: 480, display: 'flex', alignItems: 'center', gap: 8, background: 'rgba(255,255,255,0.07)', border: `1px solid ${D.navyBorder}`, borderRadius: 8, padding: '6px 12px' }}>
          <SearchRegular style={{ color: D.navyMuted, fontSize: 15, flexShrink: 0 }} />
          <input placeholder="Search quotes, policies, dealers or reports" style={{ background: 'none', border: 'none', outline: 'none', color: '#CBD5E1', fontSize: 12, width: '100%' }} />
        </div>

        <div style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 10 }}>
          {/* Notification bell */}
          <div style={{ position: 'relative' }}>
            <button
              onClick={() => undefined}
              style={{ background: 'rgba(255,255,255,0.07)', border: `1px solid ${D.navyBorder}`, borderRadius: 7, padding: '5px 8px', cursor: 'pointer', color: D.navyText, display: 'flex', alignItems: 'center' }}
            >
              <AlertRegular style={{ fontSize: 16 }} />
            </button>
          </div>

          <ProfileDropdown />
        </div>
      </header>


      {/* ── Body ── */}
      <div style={{ flex: 1, display: 'flex', overflow: 'hidden' }}>

        {/* ── Sidebar ── */}
        <nav style={{ width: expanded ? SIDEBAR_W_EXP : SIDEBAR_W_COL, flexShrink: 0, display: 'flex', flexDirection: 'column', background: D.navy, borderRight: `1px solid ${D.navyBorder}`, transition: 'width 220ms cubic-bezier(0.4,0,0.2,1)', overflow: 'visible' }}>
          <div style={{ flexShrink: 0, display: 'flex', justifyContent: expanded ? 'space-between' : 'center', alignItems: 'center', padding: expanded ? '8px 12px 8px 16px' : '8px 0', borderBottom: `1px solid ${D.navyBorder}` }}>
            {expanded && <span style={{ fontSize: 10, fontWeight: 600, color: D.teal, textTransform: 'uppercase', letterSpacing: '0.08em' }}>Navigation</span>}
            <button
              onClick={() => setExpanded(v => !v)}
              style={{ background: 'transparent', border: 'none', color: D.navyMuted, cursor: 'pointer', padding: 6, borderRadius: 6, display: 'flex', transition: 'color 120ms' }}
              onMouseEnter={e => (e.currentTarget.style.color = '#fff')}
              onMouseLeave={e => (e.currentTarget.style.color = D.navyMuted)}
            >
              <NavigationRegular style={{ fontSize: 18 }} />
            </button>
          </div>

          <div style={{ flex: 1, overflowY: 'auto', overflowX: 'visible', padding: '6px 0' }}>
            {tabs.map(tab => {
              const isActive = activeId === tab.id
              const Icon = isActive ? tab.iconFilled : tab.icon
              return (
                <NavItem
                  key={tab.id} tab={tab} isActive={isActive}
                  label={tab.label} Icon={Icon}
                  expanded={expanded} onNavigate={navigate}
                />
              )
            })}
          </div>

          {expanded && (
            <div style={{ borderTop: `1px solid ${D.navyBorder}`, padding: '12px 16px' }}>
              <div style={{ fontSize: 10, color: D.navyMuted, lineHeight: 1.6 }}>
                SageSure v2.0<br />
                <span style={{ color: D.green }}>● </span>All systems operational
              </div>
            </div>
          )}
        </nav>

        {/* ── Main content ── */}
        <main style={{ flex: 1, overflow: 'auto', padding: '24px 28px', background: D.bg }}>
          <Routes>
            <Route path="/" element={<DashboardPage onNavigate={navigate} />} />
            <Route path="/quotes" element={<QuotesPage />} />
            <Route path="/policies" element={<PoliciesPage />} />
            <Route path="/dealers" element={<DealersPage />} />
            <Route path="/reports" element={<ReportsPage />} />
            <Route path="*" element={<DashboardPage onNavigate={navigate} />} />
          </Routes>
        </main>
      </div>

    </div>
  )
}

function AppContent() {
  return (
    <LoginGate>
      <RoleGuard allow={[...PAS_APP_ROLES]}>
        <BrowserRouter><Routes><Route path="/*" element={<AppShell />} /></Routes></BrowserRouter>
      </RoleGuard>
    </LoginGate>
  )
}

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <FluentProvider theme={sageSureTheme}>
        <AuthProvider>
          <AppContent />
        </AuthProvider>
      </FluentProvider>
    </QueryClientProvider>
  )
}
