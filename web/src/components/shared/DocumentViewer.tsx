import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react'
import * as pdfjsLib from 'pdfjs-dist'
// @ts-ignore -- Vite ?url import, no type declarations for this specifier
import pdfjsWorkerUrl from 'pdfjs-dist/build/pdf.worker.mjs?url'

pdfjsLib.GlobalWorkerOptions.workerSrc = pdfjsWorkerUrl

const D = { surface: '#fff', border: '#D6E5F1', text: '#173042', sub: '#5F778C', bg: '#EEF6FB', teal: '#3D9CA2', orange: '#F7761F' }

export type DocumentViewerHandle = { jumpToPage: (page: number) => void }

/**
 * Renders the actual submitted document so a citation has somewhere to
 * land -- work order Step 4's precondition ("fix the viewer first, the
 * jump-link is meaningless without it"). PDFs render page-by-page via
 * pdf.js on a canvas; a direct image upload renders as a single page.
 * `fetchBlob` is passed in rather than a bare URL because the document
 * endpoint is auth-gated (Bearer token), same reasoning as
 * `handlers/policies.rs::get_policy_document`.
 */
const DocumentViewer = forwardRef<DocumentViewerHandle, { fetchBlob: () => Promise<Blob> }>(
  function DocumentViewer({ fetchBlob }, ref) {
    const [loading, setLoading] = useState(true)
    const [error, setError] = useState('')
    const [pdfDoc, setPdfDoc] = useState<pdfjsLib.PDFDocumentProxy | null>(null)
    const [imageUrl, setImageUrl] = useState<string | null>(null)
    const [pageNum, setPageNum] = useState(1)
    const [numPages, setNumPages] = useState(0)
    const canvasRef = useRef<HTMLCanvasElement>(null)
    const renderTaskRef = useRef<pdfjsLib.RenderTask | null>(null)

    useEffect(() => {
      let cancelled = false
      let objectUrl: string | null = null
      setLoading(true)
      setError('')
      fetchBlob()
        .then(async blob => {
          if (cancelled) return
          if (blob.type === 'application/pdf') {
            const buf = await blob.arrayBuffer()
            const doc = await pdfjsLib.getDocument({ data: buf }).promise
            if (cancelled) return
            setPdfDoc(doc)
            setNumPages(doc.numPages)
            setPageNum(1)
          } else {
            objectUrl = URL.createObjectURL(blob)
            setImageUrl(objectUrl)
            setNumPages(1)
            setPageNum(1)
          }
        })
        .catch(e => { if (!cancelled) setError(e?.message || 'Document could not be loaded.') })
        .finally(() => { if (!cancelled) setLoading(false) })
      return () => {
        cancelled = true
        if (objectUrl) URL.revokeObjectURL(objectUrl)
      }
    }, [fetchBlob])

    useEffect(() => {
      if (!pdfDoc || !canvasRef.current) return
      let cancelled = false
      renderTaskRef.current?.cancel()
      pdfDoc.getPage(pageNum).then(page => {
        if (cancelled || !canvasRef.current) return
        const viewport = page.getViewport({ scale: 1.3 })
        const canvas = canvasRef.current
        canvas.width = viewport.width
        canvas.height = viewport.height
        const ctx = canvas.getContext('2d')
        if (!ctx) return
        const task = page.render({ canvasContext: ctx, viewport })
        renderTaskRef.current = task
        task.promise.catch(() => {})
      })
      return () => { cancelled = true }
    }, [pdfDoc, pageNum])

    useImperativeHandle(ref, () => ({
      jumpToPage: (page: number) => {
        if (page < 1 || (numPages && page > numPages)) return
        setPageNum(page)
      },
    }), [numPages])

    return (
      <div style={{ display: 'flex', flexDirection: 'column', height: '100%', background: D.bg, border: `1px solid ${D.border}`, borderRadius: 10, overflow: 'hidden' }}>
        <div style={{ flex: 1, overflow: 'auto', display: 'flex', alignItems: 'flex-start', justifyContent: 'center', padding: 12 }}>
          {loading && <div style={{ padding: 40, color: D.sub, fontSize: 13 }}>Loading document…</div>}
          {error && <div style={{ padding: 40, color: '#9B2C2C', fontSize: 13 }}>{error}</div>}
          {!loading && !error && pdfDoc && <canvas ref={canvasRef} style={{ maxWidth: '100%', boxShadow: '0 1px 6px rgba(0,0,0,0.15)', background: '#fff' }} />}
          {!loading && !error && imageUrl && <img src={imageUrl} alt="Submitted document" style={{ maxWidth: '100%', boxShadow: '0 1px 6px rgba(0,0,0,0.15)' }} />}
        </div>
        <div style={{ flexShrink: 0, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 12, padding: '8px 12px', borderTop: `1px solid ${D.border}`, background: D.surface }}>
          <button disabled={pageNum <= 1} onClick={() => setPageNum(p => Math.max(1, p - 1))} style={{ border: `1px solid ${D.border}`, background: pageNum <= 1 ? '#F2F5F8' : '#fff', color: D.text, borderRadius: 6, padding: '4px 10px', fontSize: 12, cursor: pageNum <= 1 ? 'default' : 'pointer' }}>Previous</button>
          <span style={{ fontSize: 12, color: D.sub }}>Page {numPages ? pageNum : '—'} of {numPages || '—'}</span>
          <button disabled={pageNum >= numPages} onClick={() => setPageNum(p => Math.min(numPages, p + 1))} style={{ border: `1px solid ${D.border}`, background: pageNum >= numPages ? '#F2F5F8' : '#fff', color: D.text, borderRadius: 6, padding: '4px 10px', fontSize: 12, cursor: pageNum >= numPages ? 'default' : 'pointer' }}>Next</button>
        </div>
      </div>
    )
  }
)

export default DocumentViewer
