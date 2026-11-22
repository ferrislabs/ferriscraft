CREATE TABLE players (
    uuid uuid PRIMARY KEY,
    name text NOT NULL DEFAULT '',
    rank text NOT NULL DEFAULT 'guest',
    rank_seen_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE game_progress (
    uuid uuid NOT NULL,
    game text NOT NULL,
    xp bigint NOT NULL DEFAULT 0 CHECK (xp >= 0),
    played integer NOT NULL DEFAULT 0 CHECK (played >= 0),
    wins integer NOT NULL DEFAULT 0 CHECK (wins >= 0),
    losses integer NOT NULL DEFAULT 0 CHECK (losses >= 0),
    draws integer NOT NULL DEFAULT 0 CHECK (draws >= 0),
    PRIMARY KEY (uuid, game)
);

CREATE TABLE coin_ledger (
    id bigserial PRIMARY KEY,
    uuid uuid NOT NULL,
    delta bigint NOT NULL,
    reason text NOT NULL CHECK (reason IN ('match', 'cosmetic', 'admin')),
    match_id uuid,
    cosmetic text,
    admin text,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX coin_ledger_uuid_idx ON coin_ledger (uuid, id);

CREATE TABLE coin_balance (
    uuid uuid PRIMARY KEY,
    coins bigint NOT NULL CHECK (coins >= 0)
);

CREATE TABLE matches (
    match_id uuid PRIMARY KEY,
    game text NOT NULL,
    server_id text NOT NULL,
    reported_at timestamptz NOT NULL DEFAULT now(),
    ack jsonb NOT NULL
);

CREATE TABLE unlocks (
    uuid uuid NOT NULL,
    cosmetic text NOT NULL,
    source text NOT NULL,
    unlocked_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (uuid, cosmetic)
);

CREATE TABLE equipped (
    uuid uuid NOT NULL,
    slot text NOT NULL,
    cosmetic text NOT NULL,
    PRIMARY KEY (uuid, slot)
);
