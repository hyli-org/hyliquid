-- indexes to fasten building state from database

CREATE INDEX user_events_nonces_user_id ON user_events_nonces(user_id);
CREATE INDEX balance_events_user_asset_commit ON balance_events(user_id, asset_id, commit_id);