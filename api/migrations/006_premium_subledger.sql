-- Double-entry premium subledger (Phase 2)

-- Hierarchical Chart of Accounts
CREATE TABLE IF NOT EXISTS ledger_accounts (
 account_no VARCHAR(32) PRIMARY KEY,
 parent_no VARCHAR(32) REFERENCES ledger_accounts(account_no),
 name VARCHAR(128) NOT NULL,
 account_type VARCHAR(16) NOT NULL CHECK (account_type IN ('Asset','Liability','Equity','Revenue','Expense')),
 is_leaf BOOLEAN NOT NULL DEFAULT TRUE
);

-- Seed default CoA
INSERT INTO ledger_accounts (account_no, parent_no, name, account_type, is_leaf) VALUES
 ('1000', NULL, 'Assets', 'Asset', FALSE),
 ('1200', '1000', 'Accounts Receivable', 'Asset', FALSE),
 ('1210', '1200', 'AR Direct Bill', 'Asset', TRUE),
 ('1220', '1200', 'AR Agency Bill', 'Asset', TRUE),
 ('2000', NULL, 'Liabilities', 'Liability', FALSE),
 ('2100', '2000', 'Unearned Premium Reserve', 'Liability', TRUE),
 ('2200', '2000', 'Commissions Payable', 'Liability', TRUE),
 ('2310', '2000', 'Premium Refunds Payable', 'Liability', TRUE),
 ('4000', NULL, 'Revenue', 'Revenue', FALSE),
 ('4010', '4000', 'Administrative Fee Revenue', 'Revenue', TRUE)
ON CONFLICT (account_no) DO NOTHING;

-- Batch: groups journal entries from one policy event
CREATE TABLE IF NOT EXISTS accounting_batches (
 batch_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
 policy_id UUID NOT NULL,
 event_type VARCHAR(64) NOT NULL,
 created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Journal entries: bi-temporal (effective_date + system_date)
CREATE TABLE IF NOT EXISTS journal_entries (
 entry_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
 batch_id UUID NOT NULL REFERENCES accounting_batches(batch_id),
 effective_date DATE NOT NULL,
 system_date TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Journal lines: debits positive, credits negative, net per entry must = 0
CREATE TABLE IF NOT EXISTS journal_lines (
 line_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
 entry_id UUID NOT NULL REFERENCES journal_entries(entry_id),
 account_no VARCHAR(32) NOT NULL REFERENCES ledger_accounts(account_no),
 amount NUMERIC(18,4) NOT NULL CHECK (amount <> 0),
 currency CHAR(3) NOT NULL DEFAULT 'USD'
);

CREATE INDEX IF NOT EXISTS idx_accounting_batches_policy ON accounting_batches(policy_id, created_at);
CREATE INDEX IF NOT EXISTS idx_journal_entries_batch ON journal_entries(batch_id);
CREATE INDEX IF NOT EXISTS idx_journal_entries_effective ON journal_entries(effective_date);
CREATE INDEX IF NOT EXISTS idx_journal_lines_entry ON journal_lines(entry_id);
CREATE INDEX IF NOT EXISTS idx_journal_lines_account ON journal_lines(account_no);
