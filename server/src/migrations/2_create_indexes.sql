CREATE INDEX idx_orders_order_id ON orders (order_id);

CREATE INDEX idx_instruments_symbol ON instruments (symbol);

CREATE UNIQUE INDEX commits_commit_id_idx ON commits(commit_id);
-- CREATE UNIQUE INDEX commits_tx_hash_idx ON commits(tx_hash); -- TODO make this unique

CREATE INDEX order_events_commit_idx ON order_events(commit_id);
CREATE INDEX trade_events_commit_idx ON trade_events(commit_id);

-- indexes to fasten building state from database
CREATE INDEX user_events_nonces_user_id ON user_events_nonces(user_id);
CREATE INDEX balance_events_user_asset_commit ON balance_events(user_id, asset_id, commit_id);

-- Index on user identity
CREATE INDEX users_identity_idx ON users(identity);