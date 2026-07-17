# SagePAS API

Rust API for the standalone SagePAS web app.

Retained route groups:

- health and eventing status
- local development auth plus Entra bearer-token authorization
- products, customers, agents
- pricing estimate and rating quote
- quotes list/create/get/bind/issue/timeline
- policies list/get/versions/as-of/document
- PAS endorse/cancel/reinstate and OOS endorse

Retained crates are the API, domain/infra, pricing/rating/documents, PAS domain, policy and premium ledgers, OOS, locking, and event store/projector/bus support.

## Local Checks

```bash
cargo fmt --check
cargo check -p api
cargo test -p api
```

## Container Build

`Dockerfile.api` is intentionally rooted at `api/`:

```bash
docker build -f Dockerfile.api -t sagepas-api:local .
```
