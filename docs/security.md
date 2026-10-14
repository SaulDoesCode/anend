# Security

How the hub (`hub/`) and the app's daemon (`anend-hub --local`) protect what's in them,
what's enforced where, and the limits that are still open. Written during the review pass
(docs/next.md item 6, 14 Oct 2026).

## Threat model

- **A shared hub on the internet.** Strangers can reach every route. Members (once
  admitted) upload saves and classifiers, and run the web app on their own workspace on
  the server. Any of them may be hostile: hostile files, huge requests, cross-site tricks
  against other members, attempts to exhaust CPU, memory or disk.
- **A `--local` daemon.** One person on their own machine, with no login. The attacker is
  a web page in that person's browser that tries to reach `localhost` (CSRF, DNS
  rebinding).
- **Shared content.** Saves and classifiers from the hub reach other people's apps, so
  notes (markdown) and modules are untrusted data.

## What's enforced where

| concern | where | what |
|---|---|---|
| login | `web.rs` `who_is_asking` | session cookie (HttpOnly, SameSite=Lax, Secure when `--url` is https, 30 days, only its hash is stored) or a bearer app token (hash stored, shown once); banned users count as nobody |
| cross-site posts | `who_is_asking` | a POST off `/api/` is refused when `Sec-Fetch-Site: cross-site` or `Origin` isn't the hub's host. `/api/` takes bearer tokens only for writes (`POST /api/saves`, `POST /api/classifiers`), which other sites can't send |
| WebSocket | `appweb::socket` | same-origin check; no socket without a signed-in member (401), or for one still waiting (403) |
| the web app | `appweb::files`, `appweb::page`, `appweb::api` | outside `--local`, the page redirects to `/login?next=/app/`, its files and calls answer 401/403; `next` only goes to a path on the server |
| modes | `main::router` | routes outside `--mode` aren't routed at all (404); a build without a cargo feature has none of that half's handlers |
| joining | `web::join`, `invites.rs` | `--join invite` (a new server's default): an account needs an invite code; codes are 60 random bits, kept as SHA-256 hashes, shown once, with uses (≤100), an end (≤365 days) and revocation; a use is taken in the same transaction that checks it; wrong codes cost 10/h per address; only admins (and trusted members if allowed) make them |
| visibility | `store::may_see`, `Save::listed` | private: only the owner (not other members, not admins, not the API without the owner's token), pages and downloads 404 otherwise; unlisted: anyone with the id, never listed; lists, searches, feeds, profiles, the leaderboard and counts only hold public approved items; points, stars and download counts only for listed ones; `--share-quota` counts every visibility |
| logs | `ops::log_requests` | method, path, status, time; no addresses, names or queries; `/login/{secret}` and `/invite/{code}` logged without the secret |
| `--local` and DNS rebinding | `web::local_host_ok` | a local hub answers only when Host is `localhost`/`*.localhost`, an IP address, or its `--url`'s host; it warns at start when `--addr` isn't loopback |
| framing, sniffing | `web::guard_headers` | `X-Frame-Options: DENY`, `frame-ancestors 'none'`, `nosniff`, `Referrer-Policy: same-origin` on every answer |
| scripts in the web app | `web::APP_CSP` | `script-src 'self'` (no inline scripts; `hub/web/early.js` sets the theme before paint) |
| markdown | `hub/web/markdown.js` | text escaped first; link and image URLs only http(s), mailto or relative, after dropping the control characters and spaces a browser ignores (test: `node hub/web/test/markdown.mjs`); the hub's `/guide` renderer escapes raw HTML and allows only its own anchors, the repository and https |
| hub pages | askama templates | auto-escaped; the only `|safe` is the guide's own rendered HTML |
| admin | `need_admin` | non-admins get a 404; `/admin/server` (names files on the machine) only for a loopback connection carrying no forwarding header |
| login guessing | `auth::Limiter`, `web::login` | 10 logins / 10 min, 5 joins / h, 20 uploads / h per address; argon2; an unknown name costs the same time as a wrong password |
| uploads | `hub.rs` `accept`, `clshub.rs` | `--max-upload` (64 MB), checked on a copy in the hub's own folder; redb opened in `catch_unwind` on a blocking thread; modules read strictly (`clsmodule::read`, sizes checked before allocating, checksum, shapes capped in `classify/io.rs`, windows may overlap at most 3/4) |
| ids and names | `store::id_ok`, `app::clean`, `guide::picture_name` | ids are lowercase letters and digits before any path is made; save names have no separators and no leading dots; workspaces are `users/<lowercased member name>` (names are letters, digits, `-`, `_`) |
| app calls | `appweb.rs` | no body read without an owner; at most 4 calls at once per member (more wait) over at most 8 sockets; JSON bodies and frames up to 64 MB |
| disk | `app::Workspace::room` | on a shared hub `--quota` (1 GB) and 200 saves per member; past it only reading and deleting calls work |
| training | `classify.rs`, `classifiers::prepare` | threads ≤ cores, batch ≤ 4096, lr in (0, 1], epochs ≤ 1000, rounds ≤ 50; on a shared hub 2 jobs (training or indexing) per member, cores-many (4 at least) per hub, a training stops after 2 hours; steering checked even when only measured |
| filters | `core/src/filters.rs` | at most 256 conditions and 12 levels of presets, loops refused (when saved, and a bundle's presets checked for size when imported); each preset is checked and run once however often it's referred to (it was exponential: 200 references, ten deep), and a loop brought in by a bundle stops with an error |
| sizes in calls | `app.rs`, `classify.rs` | forge ≤ 32 ingredients, the lists grid only up to 1000 lists, spreads ≤ 2000 texts, scans ≤ 20 000 texts, tints ≤ 5000 keys, limits clamped |

## Known limits (not fixed)

- **Unlisted content isn't reviewed (accepted).** Anyone with an account can share a file
  by link without an admin looking. Admins see every unlisted item in the admin panel and
  can remove it; invite-only joining limits who can do this.
- **Admins can't inspect private items (by design).** They see the name, owner and size
  and can remove one, but not open it.

- **redb on hostile files (accepted).** redb isn't hardened against crafted files. The hub
  opens uploads on a copy, on a blocking thread, inside `catch_unwind`, so a panic is a
  refused file; a crafted file that makes redb loop or allocate a lot would cost that
  one thread or memory. Saves downloaded from a hub are opened by the desktop app the
  same way.
- **Changing a password or email needs no current password (accepted, low).** Anyone
  holding a live session (a borrowed computer) can change them. Cross-site forms can't,
  thanks to the origin check.
- **Unicode names (accepted, low).** Names may use any letters, so look-alike names are
  possible. Workspaces are keyed by the lowercased name, so case differences can't share
  one.
- **JSON memory (accepted).** A 64 MB JSON body becomes a larger `serde_json::Value`
  while it's handled. The 4-calls-per-member limit and the login requirement bound how
  much one member can hold at once.
- **Rate limits are per address**, kept in memory and cleared past 50 000 entries; behind
  a proxy they need `--behind-proxy` (and then `X-Forwarded-For` is trusted, so the
  proxy must set it).
