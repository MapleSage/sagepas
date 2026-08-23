# CLAUDE.md

## 🔴 ACTIVE CONSOLIDATION DECISION (2026-08-23) — read this before touching FNOL, UW, or this repo's pipeline

**This is a confirmed architecture decision, not open for relitigation.** Written here
specifically so it survives a session/model change — if you are a fresh session reading
this, treat the decision below as settled and pick up execution from the plan file, do
not re-derive or re-argue it. Mirrored in `sagesure-us/CLAUDE.md` — keep both in sync.

**Systems in scope, named precisely:** `SagePAS` standalone — this repo, Rust/Axum,
live at `pas.sagesure.io`, own Postgres DB `sagepas` on
`pg-openclaw-cid.postgres.database.azure.com` — is the target/canonical system this
decision converges onto. `app.sagesure.io` (separate repo `sagesure-us`, AKS,
`sagesure-us-insurance-api`, own Postgres DB `insurance`) and the two standalone
**FNOL & UW ACA deployments** (`fnol.sagesure.io` on `ca-azdockmgmt5ppjq-*` in
`sageinsure-rg`; `uw.sagesure.io`, likely `uw-workbench-api`/`uw-workbench-worker`,
DNS binding unconfirmed as of this writing) are the systems being folded in.

**The decision, verbatim as given:**

> Consolidation decision: FNOL and UW converge onto SagePAS. Move the Rust crates and
> API surface to write to the sagepas DB — the `fnol_submissions`, `uw_jobs` and
> `fnol_events` tables already scaffolded there are the landing zone.
>
> One shared document pipeline, two domain configurations. Ingest, OCR, extraction,
> validation, confidence scoring and human-in-loop are one engine. FNOL and UW differ
> only in field sets, rules, appetite and output object — claim/ticket versus
> submission/deal. Do not fork the pipeline; do not merge the surfaces. The two HubSpot
> cards stay two cards.
>
> Retain FNOL's ingestion breadth (images, photos, scans, not PDF-only) and UW's
> processing speed. That combination is the point of the merge.
>
> Scope before building: where do the actual documents live after this? FNOL currently
> uses Cosmos. Postgres holds metadata well and blobs badly — so decide explicitly
> whether documents move to blob storage with sagepas holding references, and say so
> rather than defaulting. **[Resolved: yes, blob storage with references — this repo's
> schema (migrations 009-011) already has `blob_container`/`blob_name` columns on
> `fnol_submissions`/`uw_jobs`, this was decided before this note was written.]**
>
> Also settle in the same scope what happens to the two now-redundant ACA deployments,
> so the saving is actually realized rather than left running. Two additions to the
> consolidation scope, both previously requested and missed.
>
> 1. FNOL and UW front-end pages render inside SagePAS. SagePAS becomes the single
>    B2B2C door using the existing profile-based access. B2C profile: quote, buy, file
>    a claim, view claim status. B2B/staff: full agent and policy administration.
>    Underwriting: demo profile only, not a customer-facing surface. The goal is that a
>    complete walkthrough — anonymous quote through purchase through claim — runs at
>    one URL with no subdomain hops.
> 2. Extraction and analysis: standalone UW is the reference implementation, not
>    `app.sagesure.io`. This is explicit and not negotiable in the merge. Requirements:
>    - Every pipeline stage individually visible and inspectable — ingestion, OCR,
>      extraction, validation, scoring, review. Not a black box between document-in
>      and score-out.
>    - Analysis grounded in the knowledge base with the detail retained, not summarized
>      away. Each conclusion carries the KB material it rests on.
>    - Where `app.sagesure.io`'s current chain is less detailed than standalone UW's,
>      UW's wins. Do not merge toward the more convenient implementation.
>
> Acceptance for (2): open any completed assessment and trace every stated conclusion
> back through its stage to the source document region and the KB passage that grounds
> it. If any step can't show its working, that step isn't done.

**Full implementation plan:**
`/Users/parvind/.claude/plans/abstract-yawning-pinwheel.md` — phased, with concrete
file/module targets and verified current-state findings on both the sagesure-us native
path and the standalone UW reference implementation. Approved by Parvind 2026-08-23.
Execution was on Phase 1 (new `api/crates/doc-pipeline` crate in this repo) as of this
note — check the plan file and this repo's git log for actual progress before assuming
where it's at.

---

# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working on HubSpot components

IMPORTANT: IF THE 'HubSpotDev' MCP SERVER IS INSTALLED USE THE TOOLS BEFORE TRYING TO MANUALLY USE CLI COMMANDS OR BEFORE TRYING TO DO ANYTHING WITH HUBSPOT ASSETS

## HubSpot Project Information
- The project configuration is in the `hsproject.json` file
- A directory is considered a part of the project if it or a directory above it contains a `hsproject.json` file
- The project src directory is defined in the `srcDir` field in the `hsproject.json`
- The project's platform version is defined in `platformVersion` in the `hs project.json`
- The `platformVersion` determines what features the project has access to as well as the shape of the configuration files

## Local Development
### Local Development Server (`hs project dev`)
- Start a local development server with `hs project dev` to view extension changes without refreshing
- The server runs on your local machine and syncs changes to HubSpot in real-time
- When the server is running, UI extensions (cards, settings pages) display a "Developing locally" tag
- Saving changes to JSX files automatically refreshes the page

### Local Proxy Configuration (`local.json`)
- During local development, you can proxy `hubspot.fetch()` requests to a locally running backend
- Create a `local.json` file in the same directory as your app's `*-hsmeta.json` file
- The proxy configuration maps HTTPS URLs to local URLs:
  ```json
  {
    "proxy": {
      "https://example.com": "http://localhost:8080"
    }
  }
  ```
- **Important**: Proxy URLs must be valid HTTPS URLs (the key, not the value)
- Path-based routing is NOT supported (e.g., `"https://example.com/a": "http://localhost:8080"` will not work)
- When a `local.json` file is detected, the CLI confirms the proxy is active
- To disable the proxy, rename the file to `local.json.bak` and restart the dev server

### Request Signing with CLIENT_SECRET
- You can inject the `CLIENT_SECRET` environment variable when starting the local dev server:
  ```shell
  CLIENT_SECRET="abc123" hs project dev
  ```
- This enables request signing during local development for testing secure backend communications

## npm packages
### `@hubspot/ui-extensions`
- In the `@hubspot/ui-extensions` npm package, only the component properties defined by the component are valid.  `style` properties are not valid

### `hubspot.fetch` API
- `hubspot.fetch` is a function provided by `@hubspot/ui-extensions` for making HTTP requests from UI components
- **Critical**: `hubspot.fetch` requires fully qualified domain names (FQDN) with HTTPS - relative paths are NOT supported
- All URLs must be added to the `permittedUrls.fetch` array in the app's `*-hsmeta.json` configuration file
- Example:
  ```json
  "permittedUrls": {
    "fetch": ["https://api.example.com", "https://api.hubapi.com"],
    "iframe": [],
    "img": []
  }
  ```
- Fetch URLs must be valid HTTPS URLs and cannot be `localhost`
- To call a local backend during development, use the `local.json` proxy configuration (see Local Development section)

## Component Information
### General
- Component configuration files must end with `-hsmeta.json`
- The `uid` field in the `-hsmeta.json` files must be unique with the project
- The `type` field in the `-hsmeta.json` files defines the type of the component
- Components can not be in nested subdirectories, only the specified directories in their corresponding component rules.
- Example components can be found in https://github.com/HubSpot/hubspot-project-components. The directories are split up by platform version and follow this format `${platformVersion}/components`. Note the project create tool only supports platform versions >= 2025.2.
- All component subdirectories must be in the project source directory

### app component
- There can only be one `app` component
- `app` component must be in the `app` directory
- If the `config.distribution` field is set to `marketplace`, the only valid `config.auth.type` value is `oauth`

### card
- `card` components must be in the `app/cards` directory
- The global `window` object is not available in the `card` component
- Cannot use `window.fetch`, and instead must use the `hubspot.fetch` function provided by the `@hubspot/ui-extensions` npm package.  Any urls called with the `hubspot.fetch` function must be added to the `config.permittedUrls.fetch` array in the `app` component's hsmeta.json file
- `hubspot.fetch` requires fully qualified HTTPS URLs (e.g., `https://api.example.com/endpoint`) - relative paths like `/api/endpoint` are NOT supported
- Only components exported from the `@hubspot/ui-extensions` npm package can be used in `card` components

#### Available Hooks for Card Components

Prefer hooks over `hubspot.fetch` — use hooks to access CRM data and extension context before falling back to `hubspot.fetch` for external HTTP requests. Hooks must be called at the component level, not inside conditionals or loops. The list below may not be exhaustive — refer to the [hooks documentation](https://developers.hubspot.com/docs/apps/developer-platform/add-features/ui-extensions/ui-extensions-sdk/hooks.md) as the source of truth for all available hooks and their parameters.

**Universal hooks** (available across all extension points):
- `useExtensionApi` - Access both context and actions from a single hook
- `useExtensionContext` - Access contextual information about the extension environment (portal, user, extension metadata)
- `useExtensionActions` - Access all available actions for the current extension point
- `useCrmSearch` - Search CRM records
- `useDebounce` - Debounce a rapidly-changing value

**CRM-specific hooks** (available in `crm.record.tab`, `crm.record.sidebar`, `crm.preview`, `helpdesk.sidebar` extension points):
- `useCrmProperties` - Fetch properties from the current CRM record
- `useAssociations` - Fetch associated CRM records

#### Available Actions for Card Components

Access actions via the `useExtensionActions` hook or the `actions` parameter from `hubspot.extend()`. The list below may not be exhaustive — refer to the [actions documentation](https://developers.hubspot.com/docs/apps/developer-platform/add-features/ui-extensions/ui-extensions-sdk/actions.md) as the source of truth for all available actions and their parameters.

**Universal actions** (available across all extension points):
- `addAlert` - Display an alert banner
- `reloadPage` - Reload the current page
- `copyTextToClipboard` - Copy text to clipboard; requires explicit user interaction
- `closeOverlay` - Close an open overlay or modal by its id
- `openIframeModal` - Open a URL in an iframe modal

**CRM-specific actions** (available in `crm.record.tab`, `crm.record.sidebar`, `crm.preview`, `helpdesk.sidebar` extension points):
- `fetchCrmObjectProperties` - Fetch property values from the current CRM record
- `refreshObjectProperties` - Refresh CRM record properties in the UI without a full page reload
- `onCrmPropertiesUpdate` - Subscribe to UI-level changes to CRM properties

#### Context Object

Access context via the `useExtensionContext` hook or the `context` parameter from `hubspot.extend()`. The list below may not be exhaustive — refer to the [context documentation](https://developers.hubspot.com/docs/apps/developer-platform/add-features/ui-extensions/ui-extensions-sdk/context.md) as the source of truth for all available context fields.

**Universal fields** (available on all extension points):
- `location` - Extension point identifier
- `portal.id` / `portal.timezone` / `portal.dataHostingLocation` - Account info
- `user.id` / `user.email` / `user.firstName` / `user.lastName` / `user.locale` / `user.language` / `user.teams` / `user.permissions` - User info
- `variables` - Project configuration variables

**CRM-specific fields** (available in `crm.record.tab`, `crm.record.sidebar`, `crm.preview`, `helpdesk.sidebar` extension points):
- `crm.objectId` - Current CRM record's ID
- `crm.objectTypeId` - Record type ID
- `extension.appId` / `extension.appName` / `extension.cardTitle` - Extension metadata

#### Logging

Use the `logger` API to send custom log messages. In local development mode, logs go to the browser console only; in production they are sent to HubSpot and viewable via `hs project logs`. The list below may not be exhaustive — refer to the [logging documentation](https://developers.hubspot.com/docs/apps/developer-platform/add-features/ui-extensions/ui-extensions-sdk/logging.md) as the source of truth for all available logging methods.

- `logger.info` - Informational messages
- `logger.debug` - Debug messages
- `logger.warn` - Warning messages
- `logger.error` - Error messages

### app-event
- `app-event` components must be in the `app/app-events` directory

### app-object
- `app-object` components must be in the `app/app-object` directory

### app-function
- `app-function` components must be in the `app/functions` directory
- `app-function` components are not available when `config.distribution` is set to `marketplace` in the `app` component `-hsmeta.son` file

# settings
- There can only be one `settings` component
- `settings` components must be in the `app/settings` directory
- The global `window` object is not available in the `settings` component
- Cannot use `window.fetch`, and instead must use the `hubspot.fetch` function provided by the `@hubspot/ui-extensions` npm package.  Any urls called with the `hubspot.fetch` function must be added to the `config.permittedUrls.fetch` array in the `app` component's `hsmeta.json` file
- `hubspot.fetch` requires fully qualified HTTPS URLs - relative paths are NOT supported
- Only components exported from the `@hubspot/ui-extensions` npm package can be used in `settings` components
- React Components from `@hubspot/ui-extensions/crm` cannot be used in `settings` components

# scim
- There can only be one `scim` component
- `scim` components must be in the `app/scim` directory

# webhooks
- There can only be one `webhooks` component.
- `webhooks` components must be in the `app/webhooks` directory

### workflow-actions
- `workflow-action` components must be in the `app/workflow-actions` directory

## HubSpot CLI commands
- All the commands and subcommands have a `--help` argument that provides details on the command and it's arguments
- The help output is standard yargs output
- The commands for working with projects in HubSpot are subcommands of `hs project`
- Debugging flag that can be added to `hs` commands and subcommands: `--debug`
- Debugging problems with CLI installation: `hs doctor`

### Project Commands
- `hs project create` - Create a new HubSpot project interactively
- `hs project upload` - Upload the project to HubSpot (build is created automatically)
- `hs project deploy` - Deploy a specific build of the project to make it live
- `hs project dev` - Start a local development server for real-time development of UI extensions
- `hs project watch` - Watch for file changes and automatically upload them
- `hs project list` - List all projects in the account
- `hs project download` - Download a project from HubSpot to local
- `hs project open` - Open the current project page in the browser
- `hs project logs` - View logs for deployed projects
- `hs project list-builds` - List all builds for a project
- `hs project validate` - Validate project configuration files
- `hs project migrate` - Migrate a project to a newer platform version
- `hs project migrate-app` - Migrate a legacy app to the projects framework
- `hs project clone-app` - Clone an existing app configuration

### Account Management
- `hs init` - Initial setup of the hubspot configuration file
- `hs account auth` - Authenticate a new account (requires browser interaction)
- `hs account list` - List all configured accounts
- `hs account use` - Switch the default account
- `hs account info` - Display information about an account
- `hs account rename` - Rename an account in the config
- `hs account remove` - Remove an account from the config
- `hs account clean` - Clean up invalid/expired authentication
- `hs account create-override` - Create a project-specific account override
- `hs account remove-override` - Remove a project-specific account override

### CMS Commands
- `hs cms upload <src> <dest>` - Upload files to HubSpot
- `hs cms fetch <src> <dest>` - Download files from HubSpot
- `hs cms watch <src> <dest>` - Watch for changes and automatically upload
- `hs cms list <path>` - List remote files in HubSpot
- `hs cms delete <path>` - Delete files from HubSpot
- `hs cms mv <srcPath> <destPath>` - Move/rename files in HubSpot
- `hs cms function list` - List all serverless functions
- `hs cms function logs <path>` - View logs for a serverless function
- `hs create template <name>` - Create a new template
- `hs create module <name>` - Create a new module
- `hs create function <name>` - Create a new serverless function
- `hs theme preview` - Preview a theme locally at https://hslocal.net:3000/

### Sandbox Management
- `hs sandbox create` - Create a development sandbox account
- `hs sandbox delete` - Delete a sandbox account

### Secrets Management
- `hs secret list` - List secrets for serverless functions
- `hs secret add <name> <value>` - Add a secret
- `hs secret update <name> <value>` - Update a secret
- `hs secret delete <name>` - Delete a secret

### Test Account Management
- `hs test-account create` - Create a configurable test account
- `hs test-account delete` - Delete a test account
- `hs test-account import-data` - Import test data

---

# WORK ORDER — 2026-08-02 — PAS underwriting, catalog, and app.sagesure.io wiring

**From:** Claude (Cowork), at Parvind's direction. **Owner: CC, this repo (`sagepas`) only.** Do not touch `sagesure-us`, `sagesure-india`, or the standalone FNOL/UW repos from this work order — each CC is scoped to its own repo.

**Context that matters before you start.** `sagepas` IS the PAS. It is all Rust — zero `.sln`/`.csproj` anywhere, zero "Happy"-branded products. Any statement you encounter (in older docs, other repos' shared memory, or a prior session's notes) describing PAS as .NET-proxied with a "Happy House/Farm/Driver" catalog is describing **stale branches of `sagesure-us`**, not this repo. Do not re-derive that conclusion; do not act on it.

All three items below are wanted. Order matters — item 1 first, because it is currently one `git checkout` away from being lost.

## Item 1 — Commit, build, and deploy the underwriting work that already exists uncommitted

**State verified 2026-08-02 via `git status` and direct file read:**
- `api/crates/rating/src/underwriting.rs` — **untracked (`??`)**, 450 lines.
- `api/crates/rating/src/lib.rs` — **modified**, declares `pub mod underwriting;` (line 3) and `pub use underwriting::*;` (line 7).
- `api/crates/rating/src/pas.rs` — **modified**, imports `evaluate_auto`/`AutoRiskProfile`/`UnderwritingDecision` (line 4), calls `evaluate_auto(&profile)` (line 111), maps to `RatingDecision::Declined` (:115), `Referred` (:123), `Quoted` (:135), and compounds `underwriting.risk_multiplier` into the final premium (:133).

This is real, wired logic — four deterministic factor evaluators (`evaluate_age`, `evaluate_prior_claims`, `evaluate_coverage_ratio`, `evaluate_vehicle_value_band`) with a `Declined` > `Referred` > `Quoted` precedence in `evaluate_auto` (:296). It is **not** a pass-through and **not** a skeleton. Any note claiming `PasProvider::rate()` always returns `Quoted`, or that `underwriting.rs` is unreferenced, is describing an earlier state and is wrong as of this date.

**Do:**
1. `cargo test -p rating` and confirm it passes. **This has NOT been verified** — the Cowork session that wrote this work order had no `cargo` binary and could only read the code. The in-file tests assert the negative paths explicitly (`three_prior_claims_is_declined`, `over_insurance_is_declined`, `underage_driver_is_declined`, `two_prior_claims_is_referred_not_declined`, `decline_wins_over_refer_when_both_present`, `missing_age_is_referred_not_silently_accepted`). If any fail, fix before committing — do not commit red tests.
2. Commit all three files together with a message naming what it does (real Auto underwriting decisioning wired into the PAS rating provider).
3. Build and deploy to `pas.sagesure.io`.

**Verification gate — none of these alone count as done:** a green `cargo test`, a healthy pod, or `/health` returning 200. Required proof: **one real quote through the live API that returns `Declined`, and one that returns `Referred`**, with the request/response retained. These enum variants sat structurally unreachable for months; showing an approval works faster proves nothing. Show the "no".

## Item 2 — Extend underwriting beyond Auto

`pas.rs:78-80` documents that non-auto lines are promoted as-is pending underwriting coverage. **That pass-through is the remaining rubber-stamp surface** — property, life and health quotes currently receive no risk evaluation at all.

Extend the `underwriting.rs` pattern to the other lines. Keep the existing design properties — they are the correct ones and were hard-won:
- **Deterministic factor tables and thresholds, not an LLM call.** Do not introduce "ask a model for a risk score." A model call that looks like reasoning but has no auditable logic behind it is the exact failure mode this estate has repeated (see the `risk-scorer` "best-effort KB scrape" and the `risk_score = 0` silent-fallback bug documented in `sagesure-us`).
- **Missing data must refer, never silently accept** — mirror `missing_age_is_referred_not_silently_accepted`.
- **`Declined` must beat `Referred` when both fire** — mirror `decline_wins_over_refer_when_both_present`.
- Each new line needs its own negative-path tests asserting a real decline and a real refer.

Do one line at a time, tested and committed, rather than all three at once.

## Item 3 — Complete the product catalog

**Current state, verified by reading the migrations:** 6 products exist across 2 migrations.
- `api/migrations/001_insurance.sql:98-102` — Auto US/USD, Life US/USD, Auto IN/INR, Life IN/INR.
- `api/migrations/007_policy_workspace.sql:55-63` — Auto AE/AED, Property AE/AED (with the correct comment: "UAE/AED products are first-class catalog records, not a display-only currency option" — preserve that principle).

**Target:** 4 lines (auto, life, health, property) × 3 markets (US/USD, AE/AED, IN/INR) = 12. **6 exist, 6 are missing:** health-US, property-US, health-IN, property-IN, health-AE, life-AE.

Add them in a new migration following 007's idempotent `WHERE NOT EXISTS` pattern — do not edit 001 or 007 in place, they are already applied. Each row is a first-class catalog record with its own `country`/`currency`, not a display-time currency conversion.

**Then repoint `app.sagesure.io`'s PAS surface at this system.** The auth difference is real: `sagesure-us` authenticates with self-issued HS256 (`middleware.rs`, symmetric shared secret, no JWKS/issuer/audience check) while `sagepas` validates Entra-issued RS256 against Microsoft's JWKS.

### 🔒 DECIDED 2026-08-02 by Parvind — option (b), Entra wholesale. Do not re-open.

`app.sagesure.io` moves to Entra ID authentication, matching `sagepas` (and the India platform). **No token-translation proxy is to be built.**

Rationale, recorded so it isn't re-litigated:
- **There are zero active users on `app.sagesure.io`** (confirmed by Parvind, 2026-08-02), so the usual objection to (b) — "every user's login changes" — has no cost here.
- A token-translation proxy in its cheap form (service-principal exchange) would make every request arriving at `sagepas` carry a single service identity instead of the real user. `sagepas` has `PasRole::Underwriter` and bitemporal policy/premium ledgers — i.e. an audit trail whose entire purpose is answering *who bound this policy*. Collapsing all users into one principal is a correctness defect in exactly the system where attribution is the product.
- The alternative proxy form (on-behalf-of flow) preserves per-user identity but requires the user to already hold an Entra identity — at which point you have moved to Entra anyway and (a)'s only advantage disappears.
- `sagesure-us`'s current HS256 scheme is not merely *different* from Entra, it is materially weaker (symmetric shared secret, no JWKS, no issuer/audience validation). Option (a) would have preserved that as a standing liability ahead of real-customer due diligence.

**Scope note for whoever picks this up:** the `sagesure-us` side of this change (frontend login + `middleware.rs` validation) belongs to the `sagesure-us` CC, not this repo. Each CC is scoped to its own repo — do not make cross-repo edits from here. This repo's side is only to confirm `sagepas` accepts the Entra tokens that `app.sagesure.io` will begin presenting, and that role claims map correctly onto `PasRole`.

## Standing rules for this work order

- **No new `.md` files.** This estate has a documented failure log spanning 70+ orphaned status/summary docs. Updates go in this `CLAUDE.md`.
- **Never call something done without runtime proof.** Build passing, pod ready, `/health` 200, and unit tests green are all necessary and none are sufficient.
- **Commit as you go.** The single highest-risk thing found in this repo today was 450 lines of finished, working underwriting logic sitting untracked.

---

## General
- Follow existing patterns in the codebase
- Use proper component structure based on component `type` in the `-hsmeta.json` file
- Ensure configuration files follow HubSpot naming conventions
- Always validate that components are placed in correct directories
- When working with UI extensions, remember that `hubspot.fetch` requires HTTPS URLs in `permittedUrls.fetch`
- Use `hs project dev` for iterative development of cards and settings pages
- Use `local.json` to proxy API requests to a local backend during development
