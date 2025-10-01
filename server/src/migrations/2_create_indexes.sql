CREATE INDEX idx_orders_order_id ON orders (order_id);

CREATE INDEX idx_instruments_symbol ON instruments (symbol);

CREATE UNIQUE INDEX commits_commit_id_idx ON commits(commit_id);
-- CREATE UNIQUE INDEX commits_tx_hash_idx ON commits(tx_hash); -- TODO make this unique

CREATE INDEX order_events_commit_idx ON order_events(commit_id);
CREATE INDEX trade_events_commit_idx ON trade_events(commit_id);