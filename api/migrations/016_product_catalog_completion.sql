-- Complete the product catalog to the full target matrix: 4 lines (auto,
-- life, health, property) x 3 markets (US/USD, AE/AED, IN/INR) = 12 products.
-- 6 already exist (auto-US, life-US, auto-IN, life-IN from 001_insurance.sql;
-- auto-AE, property-AE from 007_policy_workspace.sql). This adds the 6 that
-- were missing: health-US, property-US, health-IN, property-IN, health-AE,
-- life-AE. Each row is a first-class catalog record with its own
-- country/currency, not a display-time currency conversion — same principle
-- 007_policy_workspace.sql already established for the AE rows.
--
-- Uses the same WHERE NOT EXISTS idempotency pattern as 007_policy_workspace.sql
-- (products has no unique constraint beyond its PK, so a bare
-- ON CONFLICT DO NOTHING as used in 001_insurance.sql does not actually
-- prevent duplicate rows on rerun — matching on name+country+currency does).
INSERT INTO products (id, name, insurance_type, country, currency, description)
SELECT gen_random_uuid(), v.name, v.insurance_type, v.country, v.currency, v.description
FROM (VALUES
    ('Health Insurance',       'health',   'US', 'USD', 'Individual and family health insurance for US residents'),
    ('Property Insurance',     'property', 'US', 'USD', 'Homeowners and business property insurance for US residents'),
    ('Health Insurance India', 'health',   'IN', 'INR', 'Individual and family health insurance for India'),
    ('Property Insurance India', 'property', 'IN', 'INR', 'Property insurance for India homes and businesses'),
    ('Health Insurance UAE',   'health',   'AE', 'AED', 'Individual and family health insurance for UAE residents'),
    ('Life Insurance UAE',     'life',     'AE', 'AED', 'Term life insurance for UAE residents')
) AS v(name, insurance_type, country, currency, description)
WHERE NOT EXISTS (
    SELECT 1 FROM products p
    WHERE p.name = v.name AND p.country = v.country AND p.currency = v.currency
);
