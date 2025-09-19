-- Assets : BTC et USDT
INSERT INTO assets (symbol, scale, step)
    VALUES ('BTC', 8, 1), -- 1 sat
    ('USDT', 6, 1),
    ('ORANJ', 0, 1),
    ('HYLLAR', 0, 1);

-- 1 micro-USDT
-- Instrument BTC/USDT
-- price_scale=2 => prix en centimes ; tick_size=5 => pas de 0.05 USDT
-- qty_step=1000 => pas de 1000 sats (0.00001000 BTC) pour les quantités d'ordre
INSERT INTO instruments (symbol, tick_size, qty_step, price_scale, base_asset_id, quote_asset_id, status)
VALUES
    ('BTC/USDT', 5, -- 0.05 USDT
        1000, -- 1000 sats
        2, -- 2 décimales pour le prix
        (
            SELECT
                asset_id
            FROM assets
            WHERE
                symbol = 'BTC'
        ),
        (
            SELECT
                asset_id
            FROM
                assets
            WHERE
                symbol = 'USDT'
        ), 
        'active'
    ),
    ('HYLLAR/ORANJ', 1,
        1, 
        0, 
        (
            SELECT
                asset_id
            FROM
                assets
            WHERE
                symbol = 'HYLLAR'
        ),
        (
            SELECT
                asset_id
            FROM
                assets
            WHERE
                symbol = 'ORANJ'
        ),
        'active'
    );

-- TODO: REMOVE
-- Deux users
INSERT INTO users (identity)
VALUES
    ('txsender@orderbook'),
    ('alice'),
    ('bob');

-- Alice a 1 000 USDT (1 000 * 10^6 = 1_000_000_000 micro-USDT)
INSERT INTO balances (user_id, asset_id, total)
SELECT
    u.user_id,
    a.asset_id,
    1000 * 1000000
FROM
    users u,
    assets a
WHERE
    u.identity = 'alice'
    AND a.symbol = 'USDT';

-- Bob a 0.05 BTC (0.05 * 10^8 = 5_000_000 sats)
INSERT INTO balances (user_id, asset_id, total)
SELECT
    u.user_id,
    a.asset_id,
    5000000
FROM
    users u,
    assets a
WHERE
    u.identity = 'bob'
    AND a.symbol = 'BTC';

