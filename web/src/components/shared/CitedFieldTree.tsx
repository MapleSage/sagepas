const D = { text: '#173042', sub: '#5F778C', teal: '#3D9CA2', border: '#D6E5F1', amber: '#F3A52E' }

/**
 * Renders `extracted_json` as-produced by doc-pipeline's evaluate stage:
 * every non-null leaf is `{value, confidence, page_number,
 * text_offset_start, text_offset_end}`. This is the "show your working"
 * requirement -- a citation is a clickable page jump next to the assertion
 * it supports, not metadata in a column nobody reads (work order Step 4).
 * `pg N` calls `onJump`; a field with no citation says so plainly rather
 * than rendering nothing (work order 10.1's condition on Step 2: an
 * uncited field must be visible, not silently absent).
 */
export default function CitedFieldTree({ data, onJump }: { data: any; onJump: (page: number) => void }) {
  if (data === null || data === undefined) return null

  if (Array.isArray(data)) {
    const items = data.filter(v => v !== null && v !== undefined)
    if (!items.length) return null
    return (
      <div style={{ display: 'grid', gap: 6 }}>
        {items.map((v, i) => <CitedFieldTree key={i} data={v} onJump={onJump} />)}
      </div>
    )
  }

  if (typeof data === 'object') {
    const isCitationLeaf = 'value' in data && 'confidence' in data
    if (isCitationLeaf) {
      const value = data.value
      if (value === null || value === undefined) return null
      const confidence = typeof data.confidence === 'number' ? data.confidence : null
      const page = data.page_number
      return (
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 10, fontSize: 12, padding: '4px 0', borderBottom: '1px solid #F0F4F8' }}>
          <span style={{ color: D.text }}>{typeof value === 'boolean' ? (value ? 'Yes' : 'No') : String(value)}</span>
          <span style={{ display: 'flex', gap: 8, alignItems: 'center', flexShrink: 0 }}>
            {confidence !== null && <span style={{ color: D.sub, fontSize: 11 }}>{Math.round(confidence * 100)}%</span>}
            {page != null ? (
              <button
                onClick={() => onJump(page)}
                style={{ fontSize: 11, fontWeight: 600, color: D.teal, background: 'none', border: 'none', cursor: 'pointer', textDecoration: 'underline', padding: 0 }}
              >pg {page}</button>
            ) : (
              <span style={{ fontSize: 11, color: D.amber }}>uncited</span>
            )}
          </span>
        </div>
      )
    }

    const entries = Object.entries(data).filter(([, v]) => v !== null && v !== undefined)
    if (!entries.length) return null
    return (
      <div style={{ display: 'grid', gap: 8, paddingLeft: 10, borderLeft: `2px solid ${D.border}` }}>
        {entries.map(([k, v]) => (
          <div key={k}>
            <div style={{ fontSize: 10, fontWeight: 700, color: D.sub, textTransform: 'uppercase', letterSpacing: '.04em', marginBottom: 2 }}>{k.replace(/_/g, ' ')}</div>
            <CitedFieldTree data={v} onJump={onJump} />
          </div>
        ))}
      </div>
    )
  }

  return <span style={{ fontSize: 12, color: D.text }}>{String(data)}</span>
}
