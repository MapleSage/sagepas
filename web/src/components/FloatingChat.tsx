// GIA -- the SageSure AI assistant. Ported from sagesure-us's
// src/components/FloatingChat.tsx, which sagepas had no equivalent of at
// all (confirmed by grep before this port existed). Simplified: no
// multi-language system (sagepas has none) and no per-tab specialized
// system prompts (FNOL/UW aren't rendered as sagepas frontend pages yet --
// that's Phase 4 of the consolidation plan, not done). Uses `pasApi`'s own
// bearer-token plumbing (`client.ts`'s tokenProvider) rather than reading
// localStorage directly, matching how every other authenticated call in
// this frontend already works.

import { useState, useRef, useEffect } from 'react'
import { pasApi } from '../api/client'
import { useActiveRecord } from '../context/ActiveRecordContext'

interface Msg { role: 'user' | 'ai'; text: string }

function MarkdownText({ text, dark }: { text: string; dark?: boolean }) {
  const color = dark ? '#fff' : '#111827'
  const lines = text.split('\n')
  const elements: React.ReactNode[] = []

  const renderInline = (line: string, key: string): React.ReactNode => {
    const parts = line.split(/(\*\*[^*]+\*\*|`[^`]+`)/g)
    return (
      <span key={key}>
        {parts.map((p, i) => {
          if (p.startsWith('**') && p.endsWith('**')) return <strong key={i}>{p.slice(2, -2)}</strong>
          if (p.startsWith('`') && p.endsWith('`')) {
            return <code key={i} style={{ background: dark ? 'rgba(255,255,255,0.15)' : '#f3f4f6', borderRadius: 3, padding: '1px 4px', fontSize: '0.9em', fontFamily: 'monospace' }}>{p.slice(1, -1)}</code>
          }
          return p
        })}
      </span>
    )
  }

  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i]
    if (line.startsWith('### ')) {
      elements.push(<div key={i} style={{ fontWeight: 700, fontSize: '0.95em', color, marginTop: 8, marginBottom: 2 }}>{renderInline(line.slice(4), `h3-${i}`)}</div>)
    } else if (line.startsWith('## ')) {
      elements.push(<div key={i} style={{ fontWeight: 700, fontSize: '1em', color, marginTop: 10, marginBottom: 3, borderBottom: `1px solid ${dark ? 'rgba(255,255,255,0.2)' : '#e5e7eb'}`, paddingBottom: 3 }}>{renderInline(line.slice(3), `h2-${i}`)}</div>)
    } else if (line.startsWith('# ')) {
      elements.push(<div key={i} style={{ fontWeight: 800, fontSize: '1.05em', color, marginTop: 10, marginBottom: 4 }}>{renderInline(line.slice(2), `h1-${i}`)}</div>)
    } else if (line.match(/^[-*•] /)) {
      elements.push(<div key={i} style={{ display: 'flex', gap: 6, marginBottom: 2, paddingLeft: 4 }}><span style={{ color: dark ? 'rgba(255,255,255,0.6)' : '#9ca3af', flexShrink: 0 }}>•</span><span style={{ color }}>{renderInline(line.slice(2), `li-${i}`)}</span></div>)
    } else if (line.match(/^\d+\. /)) {
      const num = line.match(/^(\d+)\. (.*)/)!
      elements.push(<div key={i} style={{ display: 'flex', gap: 6, marginBottom: 2, paddingLeft: 4 }}><span style={{ color: dark ? 'rgba(255,255,255,0.6)' : '#9ca3af', flexShrink: 0, minWidth: 16 }}>{num[1]}.</span><span style={{ color }}>{renderInline(num[2], `ol-${i}`)}</span></div>)
    } else if (line === '' || line === '---') {
      elements.push(<div key={i} style={{ height: 6 }} />)
    } else {
      elements.push(<div key={i} style={{ color, lineHeight: 1.6, marginBottom: 1 }}>{renderInline(line, `p-${i}`)}</div>)
    }
  }

  return <div style={{ fontSize: 13 }}>{elements}</div>
}

const DEFAULT_SYSTEM_PROMPT = 'You are SageGIA, the SageSure AI assistant for the agent and policy administration workspace. Be concise and helpful, and use markdown headings/bullets where useful.'

export function FloatingChat({ activeTab }: { activeTab: string }) {
  const activeRecord = useActiveRecord()
  const [open, setOpen] = useState(false)
  const [msgsByTab, setMsgsByTab] = useState<Record<string, Msg[]>>({})
  const [input, setInput] = useState('')
  const [loading, setLoading] = useState(false)
  const endRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)

  const msgs = msgsByTab[activeTab] ?? []

  useEffect(() => {
    try {
      const raw = sessionStorage.getItem('sagegia.msgsByTab')
      if (raw) setMsgsByTab(JSON.parse(raw))
    } catch {
      // ignore storage parse errors
    }
  }, [])

  useEffect(() => {
    try {
      sessionStorage.setItem('sagegia.msgsByTab', JSON.stringify(msgsByTab))
    } catch {
      // ignore storage quota/access errors
    }
  }, [msgsByTab])

  useEffect(() => {
    if (open) setTimeout(() => inputRef.current?.focus(), 80)
  }, [open])

  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [msgs, loading])

  const appendMsg = (tabId: string, msg: Msg) => {
    setMsgsByTab(prev => ({
      ...prev,
      [tabId]: [...(prev[tabId] ?? []), msg],
    }))
  }

  const send = async () => {
    const text = input.trim()
    if (!text || loading) return
    setInput('')
    appendMsg(activeTab, { role: 'user', text })
    setLoading(true)
    try {
      const history = msgs.map(m => ({
        role: m.role === 'ai' ? 'assistant' : 'user',
        content: m.text,
      }))
      const data = await pasApi.connectChat({
        message: text,
        tab: activeTab,
        history,
        system_prompt: DEFAULT_SYSTEM_PROMPT,
        // Only attached when the open tab matches the record's own surface
        // -- e.g. don't send a stale FNOL record_id if the user has since
        // navigated to Quotes but activeRecord hasn't unmounted yet.
        record_id: activeRecord?.surface === activeTab ? activeRecord.recordId : undefined,
      })
      const reply = data?.reply ?? data?.response ?? data?.text ?? 'Sorry, something went wrong.'
      appendMsg(activeTab, { role: 'ai', text: reply })
    } catch {
      appendMsg(activeTab, { role: 'ai', text: 'Sorry, something went wrong. Please try again.' })
    } finally {
      setLoading(false)
    }
  }

  return (
    <>
      {open && (
        <div style={{
          position: 'fixed', bottom: 84, right: 24, zIndex: 1000,
          width: 360, height: 500,
          background: '#fff', borderRadius: 16,
          boxShadow: '0 8px 40px rgba(0,0,0,0.18)',
          border: '1px solid #E2E8F0',
          display: 'flex', flexDirection: 'column',
          overflow: 'hidden',
          animation: 'chatSlideUp 180ms ease',
        }}>
          <div style={{
            background: '#0D1F3C',
            padding: '12px 16px',
            display: 'flex', alignItems: 'center', justifyContent: 'space-between',
            flexShrink: 0,
          }}>
            <div>
              <div style={{ color: '#fff', fontWeight: 700, fontSize: 13, lineHeight: 1 }}>
                SageGIA · AI Assistant
              </div>
              <div style={{ color: '#93c5fd', fontSize: 11, marginTop: 3 }}>
                {activeRecord?.surface === activeTab ? `Reading ${activeRecord.surface.toUpperCase()} record ${activeRecord.recordId}` : 'SagePAS'}
              </div>
            </div>
            <button
              onClick={() => setOpen(false)}
              style={{
                background: 'rgba(255,255,255,0.1)', border: 'none',
                color: '#94a3b8', cursor: 'pointer',
                fontSize: 14, padding: '4px 8px', borderRadius: 6,
                lineHeight: 1,
              }}
            >
              ✕
            </button>
          </div>

          <div style={{
            flex: 1, overflowY: 'auto',
            padding: '14px',
            display: 'flex', flexDirection: 'column', gap: 8,
          }}>
            {msgs.length === 0 && (
              <div style={{
                textAlign: 'center', color: '#94a3b8',
                fontSize: 13, marginTop: 40, lineHeight: 1.6,
                padding: '0 16px',
              }}>
                <div style={{ fontSize: 28, marginBottom: 8 }}>💬</div>
                Ask me anything about this workspace.
              </div>
            )}
            {msgs.map((m, i) => (
              <div key={i} style={{
                display: 'flex',
                justifyContent: m.role === 'user' ? 'flex-end' : 'flex-start',
              }}>
                <div style={{
                  maxWidth: '82%',
                  padding: '9px 13px',
                  borderRadius: m.role === 'user'
                    ? '14px 14px 3px 14px'
                    : '14px 14px 14px 3px',
                  background: m.role === 'user' ? '#0D1F3C' : '#F1F5F9',
                  color: m.role === 'user' ? '#fff' : '#1e293b',
                  fontSize: 13, lineHeight: 1.55,
                  wordBreak: 'break-word',
                }}>
                  <MarkdownText text={m.text} dark={m.role === 'user'} />
                </div>
              </div>
            ))}
            {loading && (
              <div style={{ display: 'flex', justifyContent: 'flex-start' }}>
                <div style={{
                  background: '#F1F5F9',
                  borderRadius: '14px 14px 14px 3px',
                  padding: '10px 16px',
                  fontSize: 20, letterSpacing: 2, color: '#94a3b8',
                }}>
                  ···
                </div>
              </div>
            )}
            <div ref={endRef} />
          </div>

          <div style={{
            borderTop: '1px solid #E2E8F0',
            padding: '10px 12px',
            display: 'flex', gap: 8,
            flexShrink: 0,
            background: '#fafbfc',
          }}>
            <input
              ref={inputRef}
              value={input}
              onChange={e => setInput(e.target.value)}
              onKeyDown={e => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send() } }}
              placeholder="Ask GIA…"
              disabled={loading}
              style={{
                flex: 1, border: '1px solid #E2E8F0', borderRadius: 8,
                padding: '8px 11px', fontSize: 13, outline: 'none',
                color: '#1e293b', background: '#fff',
                opacity: loading ? 0.6 : 1,
              }}
            />
            <button
              onClick={send}
              disabled={loading || !input.trim()}
              style={{
                background: '#0D1F3C', color: '#fff', border: 'none',
                borderRadius: 8, padding: '8px 14px', fontSize: 12,
                fontWeight: 600, cursor: 'pointer', whiteSpace: 'nowrap',
                opacity: loading || !input.trim() ? 0.45 : 1,
                transition: 'opacity 120ms',
              }}
            >
              Send
            </button>
          </div>
        </div>
      )}

      <button
        onClick={() => setOpen(v => !v)}
        title="SageGIA — AI Assistant"
        style={{
          position: 'fixed', bottom: 24, right: 24, zIndex: 1001,
          width: 54, height: 54, borderRadius: '50%',
          background: open ? '#1A365D' : '#0D1F3C',
          border: '2px solid #2B6CB0',
          color: '#fff', fontSize: open ? 18 : 22,
          cursor: 'pointer',
          boxShadow: '0 4px 18px rgba(13,31,60,0.40)',
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          transition: 'transform 150ms ease, box-shadow 150ms ease, background 150ms ease',
        }}
        onMouseEnter={e => {
          e.currentTarget.style.transform = 'scale(1.08)'
          e.currentTarget.style.boxShadow = '0 6px 22px rgba(13,31,60,0.55)'
        }}
        onMouseLeave={e => {
          e.currentTarget.style.transform = 'scale(1)'
          e.currentTarget.style.boxShadow = '0 4px 18px rgba(13,31,60,0.40)'
        }}
      >
        {open ? '✕' : '💬'}
      </button>

      <style>{`
        @keyframes chatSlideUp {
          from { opacity: 0; transform: translateY(16px) scale(0.97); }
          to   { opacity: 1; transform: translateY(0)    scale(1);    }
        }
      `}</style>
    </>
  )
}
