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
| `POST /api/saves` | **token** (a bearer token only; a login cookie isn't enough here). `multipart/form-data` with `name`, `description`, `tags` (comma separated) and `file` (the `.redb`). Returns `{"id": "...", "status": "pending"}` (waits for an admin) or `"approved"` (trusted uploaders) |
| `GET /api/me` | **token**. `{"username": "...", "points": 120, "level": 3, "title": "scribe"}` |
| `GET /api/hub` | `{"name": "...", "description": "...", "saves": 12, "classifiers": 3, "users": 4}`: what the hub calls itself |

Uploads are at most 64 MB by default (the hub's `--max-upload`); bigger ones get a 413.

## A classifier

Trained classifiers are shared as `.anendcls` modules (core's `clsmodule`: the weights,
the class names, preset, steering, training metrics and history, and the classifiers it
builds on, bundled or named by their weights' hash).

```json
{
  "id": "q7m2kd4",
  "name": "fruit or rock",
  "author": "saul",
  "description": "tells fruit from stones",
  "tags": ["nature"],
  "classes": ["fruit", "rocks"],
  "head": "single",
  "preset": "tiny",
  "weights": 5218,
  "accuracy": 0.97,
  "macro_f1": 0.97,
  "examples": 40,
  "size": 23904,
  "upstream": [{"name": "kind", "bundled": true}],
  "stars": 4,
  "downloads": 19,
  "featured": false,
  "version": 1,
  "created": "2026-10-14T08:00:00Z",
  "updated": "2026-10-14T08:00:00Z",
  "download_url": "/classifiers/q7m2kd4/download",
  "take_url": "/app/classifiers/q7m2kd4/take"
}
```

`accuracy` and `macro_f1` are 0..1 from its last training (null when it wasn't
measured). `upstream` lists what it builds on; one not `bundled` must already be in a save
for the save to take it. `take_url` is the web app's route (docs/app-api.md).

| call | what |
|---|---|
| `GET /api/classifiers?q=&sort=&page=` | approved classifiers. `q` matches names, descriptions, authors, class names and tags (`#tag` for a tag only); `sort` and `page` as for saves. Returns `{"classifiers": [classifier...], "more": bool}` |
| `GET /api/classifiers/{id}` | one approved classifier, plus `"detail": {"module": "<its own name>", "hash": "<weights hash>", "measured_on": "..."}` |
| `GET /classifiers/{id}/download` (or `/api/classifiers/{id}/download`) | the `.anendcls` module, `Content-Disposition: attachment`. Counts a download |
| `POST /api/classifiers` | **token** (a bearer token only; a login cookie isn't enough here). `multipart/form-data` with `file` (the module), `name`, `description`, `tags`; an empty name or description is taken from the module. Returns `{"id", "status": "pending"\|"approved", "url"}` |

Modules are read strictly before anything is kept (magic, version, checksum, sizes
before allocating, every part parsed and matched to its hash, a panic is a bad file), on a
worker thread; a bad one gets a 400 saying why. They're at most 64 MB, or the hub's
`--max-upload` if smaller (a 413 when the form is bigger). Uploads count towards a limit
of 20 an hour from an address. Ids are lowercase letters and digits; anything else is a
404.
