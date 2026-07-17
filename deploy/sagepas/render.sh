#!/usr/bin/env bash
set -euo pipefail
: "${IMAGE_TAG:?IMAGE_TAG is required}"
: "${WORKLOAD_IDENTITY_CLIENT_ID:?WORKLOAD_IDENTITY_CLIENT_ID is required}"
: "${STORAGE_ACCOUNT_NAME:?STORAGE_ACCOUNT_NAME is required}"
case "$IMAGE_TAG" in latest|'') echo 'mutable/empty IMAGE_TAG is forbidden' >&2; exit 2;; esac
SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
python3 - "$SCRIPT_DIR/workloads.yaml.tmpl" "$SCRIPT_DIR/workloads.rendered.yaml" <<'PY'
import os, sys
source, target = sys.argv[1:]
text = open(source).read()
for name in ("IMAGE_TAG", "WORKLOAD_IDENTITY_CLIENT_ID", "STORAGE_ACCOUNT_NAME"):
    text = text.replace("${" + name + "}", os.environ[name])
if "${" in text:
    raise SystemExit("unresolved template placeholder remains")
open(target, "w").write(text)
PY
echo "$SCRIPT_DIR/workloads.rendered.yaml"
