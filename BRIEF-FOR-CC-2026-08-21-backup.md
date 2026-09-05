# Brief for CC — repo-level backup to NVMe (2026-08-21)

## Why this exists

Parvind is backing up the entire SageSure estate before a teardown pass on unused/orphaned Azure
infrastructure. A prior session already backed up live infra for this app: the `sagepas` database
lives on **`pg-openclaw-cid`**, not the similarly-named `pg-sagesure-india` — that took real
effort to track down (the runbook originally pointed at the wrong server) — already dumped and in
`/Volumes/SageSureBackup/sagesure-2026-08-19/db/pg-openclaw-cid-all.sql`. Also backed up:
`saopenclaw1701cid`'s `sagepas-documents` blob container, and this namespace's k8s secrets.

**This brief is about what's only safe in git.** Confirmed Rust (`api/Cargo.toml`) + Node frontend
(`web/package.json`).

## Your job

1. `git status` — flag and preserve any uncommitted or untracked work worth keeping.

2. There's an archive directory sitting alongside this repo on the same drive:
   `.sagepas-standalone-wrapper-archive-20260725`. Check what's in it — if it's an earlier
   standalone deployment attempt with commits or config not in this repo's history, copy it to
   the NVMe path below rather than assuming this repo's git history already has everything.

3. Full git bundle:
   ```bash
   git bundle create /Volumes/SageSureBackup/repo-backups/sagepas/sagepas-$(date +%Y%m%d).bundle --all
   git bundle verify /Volumes/SageSureBackup/repo-backups/sagepas/sagepas-*.bundle
   ```

4. Copy uncommitted work and the archive-directory findings to
   `/Volumes/SageSureBackup/repo-backups/sagepas/uncommitted/`.

5. Write `SUMMARY.md` into `/Volumes/SageSureBackup/repo-backups/sagepas/`.

## NVMe access

Mounted at `/Volumes/SageSureBackup`, write permissions already open on
`/Volumes/SageSureBackup/repo-backups/`. Write everything under
`/Volumes/SageSureBackup/repo-backups/sagepas/`.

## Don't

- Don't touch the live cluster or Azure resources.
- Don't skip bundle verification.
