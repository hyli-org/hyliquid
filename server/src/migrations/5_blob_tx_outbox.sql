CREATE TABLE blob_tx_outbox (
  commit_id bigint PRIMARY KEY REFERENCES commits(commit_id) ON DELETE CASCADE,
  tx_hash text NOT NULL,
  blob_tx jsonb NOT NULL,
  status text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'sent')),
  attempts integer NOT NULL DEFAULT 0,
  last_error text,
  created_at timestamptz NOT NULL DEFAULT now(),
  sent_at timestamptz
);

CREATE INDEX blob_tx_outbox_status_commit_idx
  ON blob_tx_outbox (status, commit_id);
