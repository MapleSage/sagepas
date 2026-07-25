# SagePAS isolated AKS deployment

Target: context `aks-openclaw-cid`, namespace `sagepas`, host `pas.sagesure.io`.

This deployment is additive. Never apply these resources to `sagesure-us` or `sagesure-india`.

Required before rendering/apply:

1. Unique immutable `IMAGE_TAG` pushed for both `sagepas-api` and `sagepas-web`.
2. Dedicated PostgreSQL database/user and `DATABASE_URL`; do not reuse the integrated app database.
3. User-assigned identity + federated credential for service account `sagepas/sagepas-workload`.
4. Storage Blob Data Contributor for that identity on the selected storage account.
5. Namespace secret created without committing values:

```bash
kubectl --context aks-openclaw-cid -n sagepas create secret generic sagepas-secrets \
  --from-literal=DATABASE_URL="$DATABASE_URL" \
  --from-literal=HUBSPOT_BRIDGE_SECRET="$HUBSPOT_BRIDGE_SECRET"

`HUBSPOT_BRIDGE_SECRET` must match the HubSpot project secret
`SAGEPAS_SYNC_SECRET`. Never commit either value. Preserve and merge the
existing Kubernetes Secret when rotating or adding keys.
```

Render:

```bash
IMAGE_TAG=... WORKLOAD_IDENTITY_CLIENT_ID=... STORAGE_ACCOUNT_NAME=... ./deploy/sagepas/render.sh
kubectl --context aks-openclaw-cid apply --dry-run=server -f deploy/sagepas/workloads.rendered.yaml
```

Only after validation, apply the rendered file and verify rollouts, health, Entra token claims, database rows, ingress/TLS, and unchanged existing-app baselines.
