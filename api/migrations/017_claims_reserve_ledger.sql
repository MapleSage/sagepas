-- Claims case reserve accounting (work order item 4, scoped).
-- In scope: reserve posting on claim notification, reserve movement on
-- re-estimation, both as bitemporal journal entries in the existing
-- double-entry subledger (006_premium_subledger.sql). Out of scope:
-- IBNR methodology, claims payment execution, salvage/subrogation,
-- reinsurance recoveries -- 2400 Claims Payable is seeded here as part of
-- the chart of accounts but nothing in this migration or its Rust callers
-- posts to it; it exists for the (separately scoped) payment-execution work.

INSERT INTO ledger_accounts (account_no, parent_no, name, account_type, is_leaf) VALUES
 ('2400', '2000', 'Claims Payable', 'Liability', TRUE),
 ('2410', '2000', 'Case Reserve Outstanding', 'Liability', TRUE),
 ('5000', NULL, 'Losses Incurred', 'Expense', FALSE),
 ('5010', '5000', 'Case Reserve Losses Incurred', 'Expense', TRUE)
ON CONFLICT (account_no) DO NOTHING;

-- Batches already carry policy_id (every claim belongs to a policy); claim_id
-- narrows a batch to the specific claim that caused it, alongside event_type
-- ('claim_reserve_notification' / 'claim_reserve_reestimation') -- together
-- they're how a journal batch traces back to its originating event.
ALTER TABLE accounting_batches ADD COLUMN IF NOT EXISTS claim_id UUID REFERENCES policy_claims(id);
CREATE INDEX IF NOT EXISTS idx_accounting_batches_claim ON accounting_batches(claim_id, created_at);
