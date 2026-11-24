ALTER TABLE queue_entries
    ADD COLUMN IF NOT EXISTS game text NOT NULL DEFAULT 'sheepwars'
    CHECK (game IN ('sheepwars', 'towerwars'));
