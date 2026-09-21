-- Facts saved when a creation door supplies a selected search result.
--
-- `original_publish_date` keeps the original publication date with its
-- supplied precision, separately from `publish_date` (an edition date).
-- `description_truncated` marks a provider-shortened description; it clears
-- only when an eligible replacement actually changes the description.
--
-- `work_source_references` preserves typed provider context (contributor
-- names, provider author ids, volume/ISBN/Book/Work identifiers, titles,
-- cover address intent) as source facts. Rows are never identity routes.
-- `provider` is the provider's serialized name, or 'legacy' for input that
-- named none. The primary key keeps a contributor name and its same-provider
-- author id associated by `ordinal`, so repeated names survive. The composite
-- Work foreign key is served by the unique index from migration 082.
ALTER TABLE works ADD COLUMN original_publish_date TEXT;
ALTER TABLE works ADD COLUMN description_truncated INTEGER NOT NULL DEFAULT 0;

CREATE TABLE work_source_references (
    user_id   INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    work_id   INTEGER NOT NULL,
    provider  TEXT NOT NULL,
    kind      TEXT NOT NULL,
    value     TEXT NOT NULL,
    ordinal   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (user_id, work_id, provider, kind, ordinal),
    FOREIGN KEY (user_id, work_id) REFERENCES works(user_id, id) ON DELETE CASCADE
);
