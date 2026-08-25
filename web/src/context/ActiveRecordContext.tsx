import { createContext, useContext, useEffect, useRef, useState } from 'react'

type ActiveRecord = { surface: string; recordId: string } | null

const ActiveRecordContext = createContext<{ active: ActiveRecord; setActive: (v: ActiveRecord) => void } | null>(null)

/**
 * `FloatingChat` is mounted once at the app shell (`App.tsx`), but which
 * record is "open" lives inside whichever page component is currently
 * rendered (`FnolTraceView`, `UwTraceView`, ...). This bridges the two
 * without threading a prop through every page and every route -- a page
 * calls `useSetActiveRecord('fnol', processId)` on mount and it's cleared
 * automatically on unmount, so GIA's context always reflects whatever's
 * actually on screen, not a stale value from a page the user already left.
 */
export function ActiveRecordProvider({ children }: { children: React.ReactNode }) {
  const [active, setActive] = useState<ActiveRecord>(null)
  return <ActiveRecordContext.Provider value={{ active, setActive }}>{children}</ActiveRecordContext.Provider>
}

export function useActiveRecord(): ActiveRecord {
  return useContext(ActiveRecordContext)?.active ?? null
}

export function useSetActiveRecord(surface: string, recordId: string | null | undefined) {
  const ctx = useContext(ActiveRecordContext)
  const ctxRef = useRef(ctx)
  ctxRef.current = ctx
  useEffect(() => {
    if (!recordId) return
    ctxRef.current?.setActive({ surface, recordId })
    return () => ctxRef.current?.setActive(null)
  }, [surface, recordId])
}
