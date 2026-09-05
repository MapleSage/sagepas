#!/usr/bin/env bash
# Pulls the two real secrets sagepas-api needs (AZURE_OPENAI_KEY,
# HUBSPOT_SYNC_SECRET) from the live k8s Secret into a local .env file
# for docker-compose.local.yml. Run this yourself -- it writes real
# credential values to disk.
set -euo pipefail

OUT="/Volumes/Macintosh HD Ext/sagepas/.env.local.secrets"

get_secret() {
  kubectl --context aks-openclaw-cid -n sagepas get secret sagepas-secrets \
    -o jsonpath="{.data.$1}" | base64 -d
}

{
  echo "AZURE_OPENAI_KEY=$(get_secret AZURE_OPENAI_KEY)"
  echo "HUBSPOT_SYNC_SECRET=$(get_secret HUBSPOT_SYNC_SECRET)"
} > "$OUT"

chmod 600 "$OUT"
echo "Wrote $(wc -l < "$OUT") secrets to $OUT (mode 600)."
