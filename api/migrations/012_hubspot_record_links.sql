-- Generic HubSpot object to SagePAS record links.
-- This migration intentionally stores only HubSpot record identity, never credentials or tokens.

CREATE TABLE hubspot_record_links (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    portal_id   BIGINT NOT NULL CHECK (portal_id > 0),
    object_type TEXT NOT NULL CHECK (object_type IN ('contact', 'company', 'deal', 'ticket')),
    object_id   TEXT NOT NULL CHECK (btrim(object_id) <> ''),
    customer_id UUID REFERENCES customers(id) ON DELETE SET NULL,
    quote_id    UUID REFERENCES quotes(id) ON DELETE SET NULL,
    policy_id   UUID REFERENCES policies(id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_hubspot_record_identity UNIQUE (portal_id, object_type, object_id)
);

CREATE INDEX idx_hubspot_record_links_customer
    ON hubspot_record_links(customer_id) WHERE customer_id IS NOT NULL;
CREATE INDEX idx_hubspot_record_links_quote
    ON hubspot_record_links(quote_id) WHERE quote_id IS NOT NULL;
CREATE INDEX idx_hubspot_record_links_policy
    ON hubspot_record_links(policy_id) WHERE policy_id IS NOT NULL;
CREATE INDEX idx_hubspot_record_links_updated
    ON hubspot_record_links(updated_at DESC);
