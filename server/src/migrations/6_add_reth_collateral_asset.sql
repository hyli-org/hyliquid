INSERT INTO assets (contract_name, symbol, scale, step)
VALUES ('reth-collateral', 'ORANJR', 6, 1)
ON CONFLICT (contract_name) DO NOTHING;
