// PAS role set — mirrors `domain::auth::PasRole` on the API. Used only to
// gate which UI a signed-in user sees; the API independently re-validates
// the Entra token and enforces roles server-side on every request, so a
// mismatch here can only hide/show UI, never grant real access.

export type PasRole = 'admin' | 'agent' | 'underwriter' | 'customer'

const CLAIM_TO_ROLE: Record<string, PasRole> = {
  admin: 'admin',
  'pas.admin': 'admin',
  administrator: 'admin',
  agent: 'agent',
  'pas.agent': 'agent',
  underwriter: 'underwriter',
  'pas.underwriter': 'underwriter',
  customer: 'customer',
  'pas.customer': 'customer',
  consumer: 'customer',
}

/** Maps raw Entra `roles`/`groups` claim values to PAS roles. Unknown claims are dropped, never defaulted. */
export function mapClaimsToRoles(claims: unknown[]): PasRole[] {
  const roles = new Set<PasRole>()
  for (const claim of claims) {
    if (typeof claim !== 'string') continue
    const mapped = CLAIM_TO_ROLE[claim.trim().toLowerCase()]
    if (mapped) roles.add(mapped)
  }
  return [...roles]
}

export const ROLE_LABELS: Record<PasRole, string> = {
  admin: 'Administrator',
  agent: 'Insurance Agent',
  underwriter: 'Underwriter',
  customer: 'Customer',
}
