INSERT INTO assets (contract_name, symbol, scale, step)
VALUES ('reth-collateral-orderbook', 'RETH', 6, 1)
ON CONFLICT (contract_name) DO NOTHING;
