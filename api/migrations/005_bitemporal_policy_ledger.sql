CREATE TABLE IF NOT EXISTS policy_versions (
    version_id        UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    policy_id         UUID NOT NULL,
    quote_id          UUID NOT NULL,
    customer_id       UUID NOT NULL,
    policy_number     TEXT NOT NULL,
    line_of_business  TEXT NOT NULL,
    state             TEXT NOT NULL,
    premium_cents     BIGINT NOT NULL DEFAULT 0,
    currency          CHAR(3) NOT NULL DEFAULT 'USD',
    coverage          JSONB NOT NULL DEFAULT '{}',
    -- Business period: when this version is effective in policy time
    effective_start   DATE NOT NULL,
    effective_end     DATE,
    -- System period: when this record was asserted (written)
    sys_start         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    sys_end           TIMESTAMPTZ,
    -- Sequence within policy for ordering
    version_seq       BIGINT NOT NULL DEFAULT 1,
    -- Source of this version
    source            TEXT NOT NULL DEFAULT 'rust-pas',
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_policy_versions_policy_id
    ON policy_versions(policy_id, effective_start, version_seq);
CREATE INDEX IF NOT EXISTS idx_policy_versions_current
    ON policy_versions(policy_id, sys_end)
    WHERE sys_end IS NULL;
CREATE INDEX IF NOT EXISTS idx_policy_versions_as_of
    ON policy_versions(policy_id, effective_start, effective_end);
