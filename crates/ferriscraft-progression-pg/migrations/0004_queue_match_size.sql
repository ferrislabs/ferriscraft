ALTER TABLE queue_entries
    ADD COLUMN IF NOT EXISTS match_size smallint NOT NULL DEFAULT 0;
