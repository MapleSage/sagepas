import type { ReactNode } from 'react'
import { useAuth } from './AuthProvider'
import { type PasRole } from './roles'
import AccessDenied from './AccessDenied'

/**
 * Gates `children` on the signed-in user holding at least one of `allow`.
 * Roles come only from the validated Entra ID token's `roles`/`groups`
 * claims (see `roles.ts`) — there is no client-side role picker. This is
 * a UI convenience only; the API independently enforces roles on every
 * protected request regardless of what the frontend renders.
 */
export default function RoleGuard({ allow, children }: { allow: PasRole[]; children: ReactNode }) {
  const { roles } = useAuth()
  const permitted = roles.some(role => allow.includes(role))
  if (!permitted) return <AccessDenied allow={allow} />
  return <>{children}</>
}
