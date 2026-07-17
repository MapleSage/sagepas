# SagePAS Standalone

Standalone SagePAS policy administration system. This repository contains only the Rust PAS API, the React web app, PostgreSQL migrations, and the isolated `deploy/sagepas/` deployment template.

## Layout

- `api/` - Rust workspace for the PAS API and retained domain crates
- `web/` - React/Vite SagePAS web application
- `deploy/sagepas/` - isolated Kubernetes workload template and renderer

## API

```bash
cd api
cargo fmt --check
cargo check -p api
cargo test -p api
```

Build the API container from `api/`:

```bash
cd api
docker build -f Dockerfile.api -t sagepas-api:local .
```

## Web

```bash
cd web
npm ci
npm run lint
npm run build
```

## Deployment Template

Render only with non-secret placeholders. Do not commit the rendered manifest.

```bash
IMAGE_TAG=local-test \
WORKLOAD_IDENTITY_CLIENT_ID=00000000-0000-0000-0000-000000000000 \
STORAGE_ACCOUNT_NAME=sagepasteststorage \
./deploy/sagepas/render.sh
```

Do not create remotes, push, deploy, or access cloud resources from this standalone repo without an explicit request.
