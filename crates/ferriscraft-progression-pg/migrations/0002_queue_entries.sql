CREATE TABLE queue_entries (
    seq bigserial PRIMARY KEY CHECK (seq > 0),
    player_uuid uuid NOT NULL UNIQUE,
    name text NOT NULL,
    joined_at timestamptz NOT NULL DEFAULT now(),
    state text NOT NULL DEFAULT 'waiting' CHECK (state IN ('waiting', 'dispatched')),
    server_id text,
    transfer_id uuid,
    CHECK ((state = 'dispatched') = (server_id IS NOT NULL AND transfer_id IS NOT NULL))
);

ALTER SEQUENCE queue_entries_seq_seq RESTART WITH 4294967296;
