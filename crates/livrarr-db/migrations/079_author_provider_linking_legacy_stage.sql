INSERT INTO author_route_legacy_staging (
    user_id,
    author_id,
    provider,
    raw_value,
    status,
    staged_at,
    updated_at
)
SELECT
    user_id,
    id,
    'open_library',
    TRIM(ol_key),
    'pending',
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
FROM authors
WHERE ol_key IS NOT NULL
  AND TRIM(ol_key) <> '';

INSERT INTO author_route_legacy_staging (
    user_id,
    author_id,
    provider,
    raw_value,
    status,
    staged_at,
    updated_at
)
SELECT
    user_id,
    id,
    'goodreads',
    TRIM(gr_key),
    'pending',
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
FROM authors
WHERE gr_key IS NOT NULL
  AND TRIM(gr_key) <> '';

INSERT INTO author_route_legacy_staging (
    user_id,
    author_id,
    provider,
    raw_value,
    status,
    staged_at,
    updated_at
)
SELECT
    user_id,
    id,
    'hardcover',
    TRIM(hc_key),
    'pending',
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
FROM authors
WHERE hc_key IS NOT NULL
  AND TRIM(hc_key) <> '';

INSERT INTO author_name_legacy_staging (
    author_id,
    user_id,
    name,
    status,
    staged_at,
    updated_at
)
SELECT
    id,
    user_id,
    name,
    'pending',
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
FROM authors;
