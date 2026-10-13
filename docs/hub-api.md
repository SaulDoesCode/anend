# The hub's API

What the app (and anything else) uses to talk to an anend hub. Every response is JSON
unless it's a download. Errors come back with a 4xx/5xx status and `{"error": "what went wrong"}`.

Authenticated calls send `Authorization: Bearer <token>`. A token is made on the hub's
website, on your profile page ("app tokens"), and pasted into the app's settings.

## A save

```json
{
  "id": "k3x9q2",
  "name": "the psalms",
  "author": "saul",
  "description": "every psalm verse as a memory",
  "tags": ["bible", "hebrew"],
  "notes": 12,
  "memories": 2511,
  "size": 1048576,
  "stars": 31,
  "downloads": 204,
  "featured": false,
  "created": "2026-10-13T08:00:00Z",
  "updated": "2026-10-13T08:00:00Z",
  "download_url": "/saves/k3x9q2/download"
}
```

`created`/`updated` are RFC 3339. `download_url` is relative to the hub's address.

## Calls

| call | what |
|---|---|
| `GET /api/saves?q=&sort=&page=` | approved saves. `q` matches names, descriptions, tags and authors; `sort` is `new` (default), `stars`, `downloads` or `featured`; `page` starts at 0, 24 a page. Returns `{"saves": [save...], "more": bool}` |
| `GET /api/saves/{id}` | one approved save, plus `"sample": [{"text": "...", "ordinal": 1, "reversed": 2}]` (up to 24 of its memories) |
| `GET /saves/{id}/download` | the save's `.redb` file, `Content-Disposition: attachment; filename="<name>.redb"`. Counts a download |
| `POST /api/saves` | **token**. `multipart/form-data` with `name`, `description`, `tags` (comma separated) and `file` (the `.redb`). Returns `{"id": "...", "status": "pending"}` (waits for an admin) or `"approved"` (trusted uploaders) |
| `GET /api/me` | **token**. `{"username": "...", "points": 120, "level": 3, "title": "scribe"}` |
| `GET /api/hub` | `{"name": "...", "description": "...", "saves": 12, "users": 4}`: what the hub calls itself |

Uploads are at most 64 MB by default (the hub's `--max-upload`); bigger ones get a 413.
