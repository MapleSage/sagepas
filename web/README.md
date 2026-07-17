# SageSure Standalone Policy Administration Frontend

Standalone Vite/React review frontend for the SageSure Policy Administration System (PAS). It preserves the original insurance platform's compact navigation, page layout, list/detail views, tabs, and operational modals while using only verified Rust API contracts under `/api/v1`.

## Branding

The visual treatment follows the confirmed current `app.sagesure.io` reference: navy `#0D2B3D`, secondary navy `#174D6D`, teal `#3D9CA2`, purple `#102D7B`, orange `#F7761F`, pale blue-grey `#EEF6FB`, and white cards. Headings use Montserrat and body copy uses Poppins through locally bundled `@fontsource` packages. The header shield/tree source asset is `public/sageinsure_favion.png`, copied unchanged from `frontend/public/sageinsure_favion.png`; the adjacent uppercase SAGESURE wordmark is rendered in Montserrat for the compact lockup. `public/sagesure_logo.jpeg` is also preserved unchanged from the same canonical frontend source.

## Supported native functions

- Dashboard statistics: `GET /api/v1/dashboard/stats`
- Products: `GET /api/v1/products` (bare array)
- Customers: `GET /api/v1/customers`, `POST /api/v1/customers`
- Indicative pricing: `POST /api/v1/pricing/estimate`
- Carrier rating: `POST /api/v1/rating/quote`
  - A `422` response identifying `pas:skeleton:v1` stops the quote flow visibly. The UI does not create a quote after this response.
- Quotes: list/create/detail, bind, issue, and timeline via verified `/api/v1/quotes` routes
- Policies: list/detail, bitemporal versions, and policy PDF via verified `/api/v1/policies` routes
- Quote and policy identifiers are backend UUIDs. The frontend does not generate IDs or sample records.

## Pending backend integrations

The original interfaces remain visible, but the following controls show **Backend integration pending**, are disabled for writes/downloads, and make no unsupported network calls:

- Dealer and commission CRUD
- Policy endorsement
- Policy cancellation
- Policy reinstatement
- BDX export

Policy transactions use the related quote timeline only when the policy includes a real `quote_id`. Policy documents use `GET /api/v1/policies/:id/document`. Policy issue uses `POST /api/v1/quotes/:quote_id/issue`, never a policy issue route.

## Authentication behavior

This standalone frontend never stores, refreshes, removes, or otherwise mutates authentication/business data in browser storage. For an externally authenticated host, it reads the first available token using this key order:

1. `accessToken`
2. `auth_token`
3. `access_token`
4. `token`

For each key, `sessionStorage` is checked before `localStorage`. If no token exists, the application still renders its review shell and shows truthful API empty/error states instead of redirecting to login.

## Local development

Requirements: Node.js 22+ and npm.

```bash
npm install
npm run lint
npm run build
npm run dev -- --host 127.0.0.1 --port 5180
```

Vite proxies `/api` to `http://127.0.0.1:3000` by default. Override the backend target without changing source:

```bash
VITE_API_PROXY_TARGET=http://127.0.0.1:8080 npm run dev -- --host 127.0.0.1 --port 5180
```

Open <http://127.0.0.1:5180/>. The SPA routes include `/dashboard`, `/quotes`, `/quotes/quick`, `/quotes/new`, `/quotes/:id`, `/policies`, `/policies/:id`, `/dealers`, and `/reports`.

## Container notes

`Dockerfile` builds the Vite application and serves it through unprivileged nginx on port 8080. `nginx.conf` provides SPA fallback, `/healthz`, and proxies `/api/` to `sagepas-api:3000`. This repository does not deploy, push images, or contain standalone Kubernetes deployment manifests.
