-- Durable HubSpot synchronization runtime.
--
-- The PAS writes only transactional outbox records. No database trigger calls
-- HubSpot (or the bridge), so quote/bind/issue remains independent of CRM
-- availability. Credentials and tokens are deliberately absent from this
-- schema.

CREATE TABLE hubspot_sync_outbox (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    portal_id       BIGINT NOT NULL CHECK (portal_id > 0),
    object_type     TEXT NOT NULL CHECK (object_type IN ('contact', 'company', 'deal', 'ticket')),
    object_id       TEXT NOT NULL CHECK (btrim(object_id) <> ''),
    operation       TEXT NOT NULL CHECK (operation IN ('upsert')),
    payload         JSONB NOT NULL CHECK (jsonb_typeof(payload) = 'object'),
    status          TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'dispatching', 'dispatched', 'succeeded', 'failed')),
    attempts        INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    available_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    claimed_at      TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ,
    last_error      TEXT,
    dedupe_key      TEXT NOT NULL UNIQUE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_hubspot_sync_outbox_dispatch
    ON hubspot_sync_outbox (available_at, created_at)
    WHERE status IN ('pending', 'failed', 'dispatching');
CREATE INDEX idx_hubspot_sync_outbox_identity
    ON hubspot_sync_outbox (portal_id, object_type, object_id, created_at DESC);

CREATE TABLE hubspot_sync_state (
    portal_id       BIGINT NOT NULL,
    object_type     TEXT NOT NULL,
    object_id       TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'dispatching', 'dispatched', 'succeeded', 'failed')),
    last_event_id   UUID REFERENCES hubspot_sync_outbox(id) ON DELETE SET NULL,
    attempt_count   INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    last_attempt_at TIMESTAMPTZ,
    last_synced_at  TIMESTAMPTZ,
    last_error      TEXT,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (portal_id, object_type, object_id),
    FOREIGN KEY (portal_id, object_type, object_id)
        REFERENCES hubspot_record_links(portal_id, object_type, object_id)
        ON UPDATE CASCADE ON DELETE CASCADE
);

CREATE INDEX idx_hubspot_sync_state_status
    ON hubspot_sync_state (status, updated_at DESC);

CREATE TABLE hubspot_crm_property_cache (
    portal_id        BIGINT NOT NULL,
    object_type      TEXT NOT NULL,
    object_id        TEXT NOT NULL,
    properties       JSONB NOT NULL DEFAULT '{}'::jsonb
                     CHECK (jsonb_typeof(properties) = 'object'),
    source_updated_at TIMESTAMPTZ,
    cached_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (portal_id, object_type, object_id),
    FOREIGN KEY (portal_id, object_type, object_id)
        REFERENCES hubspot_record_links(portal_id, object_type, object_id)
        ON UPDATE CASCADE ON DELETE CASCADE
);

CREATE INDEX idx_hubspot_crm_property_cache_cached
    ON hubspot_crm_property_cache (cached_at DESC);

CREATE TABLE hubspot_sync_audit (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    event_id    UUID REFERENCES hubspot_sync_outbox(id) ON DELETE SET NULL,
    portal_id   BIGINT NOT NULL CHECK (portal_id > 0),
    object_type TEXT NOT NULL CHECK (object_type IN ('contact', 'company', 'deal', 'ticket')),
    object_id   TEXT NOT NULL CHECK (btrim(object_id) <> ''),
    direction   TEXT NOT NULL CHECK (direction IN ('outbound', 'inbound')),
    outcome     TEXT NOT NULL CHECK (
        outcome IN (
            'queued', 'dispatching', 'dispatched', 'succeeded', 'failed',
            'retry_requested', 'properties_cached', 'properties_ignored'
        )
    ),
    error_code  TEXT,
    details     JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(details) = 'object'),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_hubspot_sync_audit_identity
    ON hubspot_sync_audit (portal_id, object_type, object_id, created_at DESC);
CREATE INDEX idx_hubspot_sync_audit_event
    ON hubspot_sync_audit (event_id, created_at DESC) WHERE event_id IS NOT NULL;

-- Build a bounded, credential-free CRM projection from the linked PAS rows.
-- Coverage, documents, national IDs, addresses, arbitrary JSON, and all
-- integration configuration are intentionally excluded.
CREATE FUNCTION hubspot_sync_payload(
    p_portal_id BIGINT,
    p_object_type TEXT,
    p_object_id TEXT,
    p_reason TEXT
) RETURNS JSONB
LANGUAGE sql
STABLE
AS $$
    SELECT jsonb_build_object(
        'schema_version', 1,
        'reason', p_reason,
        'portal_id', link.portal_id,
        'object_type', link.object_type,
        'object_id', link.object_id,
        'link', jsonb_strip_nulls(jsonb_build_object(
            'customer_id', link.customer_id,
            'quote_id', link.quote_id,
            'policy_id', link.policy_id
        )),
        'customer', CASE WHEN customer.id IS NULL THEN NULL ELSE jsonb_build_object(
            'id', customer.id,
            'name', customer.name,
            'email', customer.email,
            'phone', customer.phone,
            'country', customer.country,
            'currency', customer.currency
        ) END,
        'quote', CASE WHEN quote.id IS NULL THEN NULL ELSE jsonb_build_object(
            'id', quote.id,
            'customer_id', quote.customer_id,
            'product_id', quote.product_id,
            'state', quote.state,
            'premium', quote.premium,
            'currency', quote.currency,
            'created_at', quote.created_at,
            'updated_at', quote.updated_at
        ) END,
        'policy', CASE WHEN policy.id IS NULL THEN NULL ELSE jsonb_build_object(
            'id', policy.id,
            'quote_id', policy.quote_id,
            'policy_number', policy.policy_number,
            'customer_id', policy.customer_id,
            'product_id', policy.product_id,
            'state', policy.state,
            'premium', policy.premium,
            'currency', policy.currency,
            'start_date', policy.start_date,
            'end_date', policy.end_date,
            'created_at', policy.created_at
        ) END
    )
    FROM hubspot_record_links link
    LEFT JOIN customers customer ON customer.id = link.customer_id
    LEFT JOIN quotes quote ON quote.id = link.quote_id
    LEFT JOIN policies policy ON policy.id = link.policy_id
    WHERE link.portal_id = p_portal_id
      AND link.object_type = p_object_type
      AND link.object_id = p_object_id
$$;

-- Enqueue at most one snapshot per record/reason/database transaction. If
-- multiple native rows change in the same transaction, refresh that snapshot
-- in place so the worker sees the final committed state.
CREATE FUNCTION hubspot_enqueue_sync(
    p_portal_id BIGINT,
    p_object_type TEXT,
    p_object_id TEXT,
    p_reason TEXT
) RETURNS UUID
LANGUAGE plpgsql
AS $$
DECLARE
    v_event_id UUID;
    v_payload JSONB;
    v_dedupe_key TEXT;
    v_inserted BOOLEAN := FALSE;
BEGIN
    v_payload := hubspot_sync_payload(p_portal_id, p_object_type, p_object_id, p_reason);
    IF v_payload IS NULL THEN
        RETURN NULL;
    END IF;

    v_dedupe_key := concat_ws(':', p_portal_id, p_object_type, p_object_id, txid_current(), p_reason);

    INSERT INTO hubspot_sync_outbox (
        portal_id, object_type, object_id, operation, payload, dedupe_key
    ) VALUES (
        p_portal_id, p_object_type, p_object_id, 'upsert', v_payload, v_dedupe_key
    )
    ON CONFLICT (dedupe_key) DO NOTHING
    RETURNING id INTO v_event_id;

    IF v_event_id IS NULL THEN
        UPDATE hubspot_sync_outbox
        SET payload = v_payload, updated_at = NOW()
        WHERE dedupe_key = v_dedupe_key
        RETURNING id INTO v_event_id;
    ELSE
        v_inserted := TRUE;
    END IF;

    INSERT INTO hubspot_sync_state (
        portal_id, object_type, object_id, status, last_event_id, updated_at
    ) VALUES (
        p_portal_id, p_object_type, p_object_id, 'pending', v_event_id, NOW()
    )
    ON CONFLICT (portal_id, object_type, object_id) DO UPDATE SET
        status = 'pending',
        last_event_id = EXCLUDED.last_event_id,
        attempt_count = 0,
        last_attempt_at = NULL,
        last_error = NULL,
        updated_at = NOW();

    IF v_inserted THEN
        INSERT INTO hubspot_sync_audit (
            event_id, portal_id, object_type, object_id, direction, outcome, details
        ) VALUES (
            v_event_id, p_portal_id, p_object_type, p_object_id,
            'outbound', 'queued', jsonb_build_object('reason', p_reason)
        );
    END IF;

    RETURN v_event_id;
END;
$$;

CREATE FUNCTION hubspot_record_link_enqueue_trigger() RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    PERFORM hubspot_enqueue_sync(
        NEW.portal_id, NEW.object_type, NEW.object_id, 'record_link_changed'
    );
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_hubspot_record_link_enqueue
AFTER INSERT OR UPDATE OF customer_id, quote_id, policy_id
ON hubspot_record_links
FOR EACH ROW EXECUTE FUNCTION hubspot_record_link_enqueue_trigger();

CREATE FUNCTION hubspot_native_record_enqueue_trigger() RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    linked RECORD;
BEGIN
    IF TG_TABLE_NAME = 'customers' THEN
        FOR linked IN
            SELECT portal_id, object_type, object_id
            FROM hubspot_record_links WHERE customer_id = NEW.id
        LOOP
            PERFORM hubspot_enqueue_sync(
                linked.portal_id, linked.object_type, linked.object_id, 'customer_changed'
            );
        END LOOP;
    ELSIF TG_TABLE_NAME = 'quotes' THEN
        FOR linked IN
            SELECT portal_id, object_type, object_id
            FROM hubspot_record_links WHERE quote_id = NEW.id
        LOOP
            PERFORM hubspot_enqueue_sync(
                linked.portal_id, linked.object_type, linked.object_id, 'quote_changed'
            );
        END LOOP;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_hubspot_customer_enqueue
AFTER UPDATE OF name, email, phone, country, currency
ON customers
FOR EACH ROW EXECUTE FUNCTION hubspot_native_record_enqueue_trigger();

CREATE TRIGGER trg_hubspot_quote_enqueue
AFTER UPDATE OF state, premium, currency, customer_id, product_id
ON quotes
FOR EACH ROW EXECUTE FUNCTION hubspot_native_record_enqueue_trigger();

-- A newly issued policy inherits every portal-qualified mapping already tied
-- to its quote. The link update trigger then queues the complete policy
-- snapshot; no network operation occurs in the issue transaction.
CREATE FUNCTION hubspot_policy_enqueue_trigger() RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    linked RECORD;
BEGIN
    IF TG_OP = 'INSERT' THEN
        UPDATE hubspot_record_links
        SET policy_id = NEW.id, updated_at = NOW()
        WHERE quote_id = NEW.quote_id
          AND (policy_id IS NULL OR policy_id = NEW.id);
        -- The record-link update trigger enqueues the final policy snapshot.
        RETURN NEW;
    END IF;

    FOR linked IN
        SELECT portal_id, object_type, object_id
        FROM hubspot_record_links WHERE policy_id = NEW.id
    LOOP
        PERFORM hubspot_enqueue_sync(
            linked.portal_id, linked.object_type, linked.object_id, 'policy_changed'
        );
    END LOOP;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_hubspot_policy_enqueue
AFTER INSERT OR UPDATE OF state, premium, currency, start_date, end_date, customer_id, product_id
ON policies
FOR EACH ROW EXECUTE FUNCTION hubspot_policy_enqueue_trigger();

-- Existing links predate this synchronization migration. Queue one initial
-- projection for each so every mapping starts with inspectable durable state.
SELECT hubspot_enqueue_sync(
    portal_id, object_type, object_id, 'migration_backfill'
)
FROM hubspot_record_links;
