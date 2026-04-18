# Author

A person who writes books. Globally shared across all users.

## Lifecycle

1. **Created** — during work enrichment (author discovered from provider metadata)
2. **Enriched** — author metadata populated from providers
3. **Monitored** (optional) — background job polls for new works (24h interval)

## Monitoring

When an author is monitored:
- Author monitor job checks providers daily for new works
- 1-second delay between provider requests
- New works trigger notifications
- Can auto-add works based on user policy
- 429 rate limit responses create diagnostic notifications

## Relationship to Works

An author has many works. Works reference their primary author. The filesystem layout uses the author name for directory structure: `{root}/{user_id}/{Author}/{Title}`.
