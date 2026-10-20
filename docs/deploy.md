# Running an anend server

`anend-hub` is one program with two halves: the **hub** (a site for sharing saves and
classifiers, with people, stars, levels and an API the desktop app uses) and the **web
app** (the anend app in the browser, each member working on saves of their own on the
server). It runs either half or both, over one account system.

## Modes

| mode | flag | what's served |
|---|---|---|
| both | `--mode both` (default) | the hub, the web app at `/app/`, and the routes between them (taking a hub save into your workspace, sharing from the app) |
| hub | `--mode hub` | the sharing site and its API (`/api/saves`, `/api/classifiers`, …); no `/app/` |
| app | `--mode app` | the web app and its API (`/app/api/{op}`, `/app/ws`); `/` is a landing page; no saves grid, classifiers, leaderboard or hub API |
| local | `--local` | the web app for one person on their own machine: one workspace, no login, answers only to localhost, IP addresses or its `--url` |

Every mode has the accounts (`/join`, `/login`, `/u/{name}`, app tokens), the admin panel,
`/about`, `/terms`, `/privacy`, `/guide`, `/api/me` and `/health`. A route outside the
mode doesn't exist (a 404), and the pages only offer what's there. One data folder works
in any mode: switching modes keeps every account, save and setting.

### The web app inside

The web app at `/app/` is the Dioxus app's web build (`app/`), built into the hub's
binary from `app/web/dist`. Build it first (it needs the `wasm32-unknown-unknown` target
and `wasm-bindgen` at the version in Cargo.lock), then the hub:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129   # the version Cargo.lock pins
app/web/build.sh                                   # writes app/web/dist
cargo build --release -p anend-hub
```

A hub built without it still runs; its `/app/` page says to run `app/web/build.sh` and
build again. (A debug build reads `app/web/dist` from disk, so rebuilding the web app is
enough there.)

### Builds without a half

Cargo features `hub` and `app` (both by default):

```sh
cargo build --release -p anend-hub                                          # both
cargo build --release -p anend-hub --no-default-features --features app     # the web app alone
cargo build --release -p anend-hub --no-default-features --features hub     # the hub alone
```

A build without a half refuses a mode that needs it when it starts. The database code is
in every build, so a data folder moves between builds.

## Flags

| flag | default | |
|---|---|---|
| `--mode hub\|app\|both` | both | what to serve |
| `--local` | | the web app alone for one person, no login |
| `--addr HOST:PORT` | 127.0.0.1:8080 | where to listen |
| `--url URL` | `http://ADDR` | the public address, for links (login links, invites); `https://` makes cookies Secure |
| `--data DIR` | ./hub-data | the database (`hub.redb`), shared files (`saves/`, `classifiers/`), members' workspaces (`users/`), `server.json`, `terms.md`, `privacy.md` |
| `--join invite\|apply\|open` | invite (new servers) | how people get an account; kept in the database, changeable in the admin panel |
| `--open` | | same as `--join open` |
| `--admin NAME` | | make NAME an admin (created if needed) and print a one-hour login link |
| `--invite` | | print a one-use invite link, good for 7 days (needs an admin) |
| `--max-upload MB` | 64 | largest save or classifier taken |
| `--share-quota MB` | 512 | how much each member keeps on the hub, private and unlisted included (0: none) |
| `--quota MB` | 1024 | how much each member's web-app saves may fill (0: none; `--local` has none); also at most 200 saves |
| `--name`, `--about`, `--contact` | | what the pages say about the server |
| `--terms FILE`, `--privacy FILE` | DATA/terms.md, DATA/privacy.md | markdown shown at `/terms` and `/privacy` (examples in `hub/config/`) |
| `--behind-proxy` | | trust `X-Forwarded-For` (the proxy must set it) |
| `--tls-cert FILE`, `--tls-key FILE` | | serve https directly (or set them at `/admin/server`, from the machine itself) |
| `--log text\|json` | text | one line, or one JSON object, per request: method, path, status, ms (no addresses, names, queries or secrets) |
| `--no-ratelimit` | | for tests only |

## First start

```sh
anend-hub --data /var/lib/anend-hub --url https://notes.example.org --admin you
```

It prints a login link for `you`; open it and set a password on your page. Then make
invites at `/invites` (or the admin panel), or start with `--invite` to print one.
Nothing is emailed: login links and invite links are printed or shown once, and you pass
them on. A member who loses their password gets a fresh login link from an admin
(admin panel → everyone → "make a login link").

## Joining

- **invite** (default for a new server): an account needs an invite code. Admins make
  invites (for 1-100 people, lasting 0-365 days; 0 = until used up); trusted members may
  too when "trusted members may invite" is on in the admin panel. A code is shown once
  (only its hash is kept); every invite is listed in the admin panel with who used it,
  and can be taken back. Mistyped codes count against the address (10 an hour).
- **apply**: anyone may apply; an admin lets them in. An invite code skips the wait.
- **open**: anyone may join at once.

A server that had people before the policy was kept starts as `apply`, as before.

## Visibility

Every save and classifier on the hub is **private** (its owner alone: page, download,
API), **unlisted** (anyone with the link; never listed) or **public** (listed after an
admin lets it in). Only public, approved things appear in the grids, search, feeds,
profiles, the leaderboard and the API's lists. Private and unlisted ones skip the review,
earn no points and count towards `--share-quota`. Admins see every one listed in the admin
panel and can remove any, but can't open a private one. Unlisted content isn't reviewed:
if that worries you, keep `--join invite` and remove what breaks your terms.

## TLS

Either put the server behind a reverse proxy (Caddy, nginx) that terminates https, with
`--addr 127.0.0.1:8080 --behind-proxy --url https://…`, or give `--tls-cert`/`--tls-key`.
A Caddyfile is two lines:

```
notes.example.org {
    reverse_proxy 127.0.0.1:8080
}
```

The proxy must pass WebSocket upgrades for `/app/ws` (Caddy does by default; nginx needs
`proxy_set_header Upgrade $http_upgrade; proxy_set_header Connection "upgrade";`).

## Running it

- **systemd:** `hub/anend-hub.service` (a system user, the data folder its only writable
  place). Stopping sends SIGTERM; requests in flight get up to 20 s to finish.
- **Docker:** `hub/Dockerfile` (`docker build -f hub/Dockerfile -t anend-hub .`); data in
  the `/data` volume; a `HEALTHCHECK` on `/health`.
- **Health:** `GET /health` → 200 `{"status": "ok", "mode": "both", "version": "…"}`,
  503 when the database doesn't answer.

## Backups

Everything is in the data folder. For a consistent copy, stop the server (or snapshot
the filesystem), then copy the folder:

```sh
systemctl stop anend-hub && tar czf anend-$(date +%F).tgz -C /var/lib anend-hub && systemctl start anend-hub
```

`hub.redb` holds the accounts, settings, invites and the records of shared things;
`saves/` and `classifiers/` the shared files; `users/<name>/saves/` each member's web-app
saves (redb files the desktop app opens). `incoming/` is scratch and can be left out.

## As a paid service

To sell access to the web app (with or without the hub):

1. Run `--mode app` (the app alone, nothing shared between customers) or `--mode both`
   (customers also share on a hub), behind https, with `--log json` into your log system.
2. Keep `--join invite`. Each sale is an invite for one person (admin panel → make an
   invite, 1 person, a few days); the customer joins with it. To end an account, ban it
   in the admin panel (its sessions end at once).
3. Set the limits you sell: `--quota` (web-app disk per member), `--share-quota` (hub
   storage per member), `--max-upload`. A plan with more room needs its own server today
   (quotas are per server, not per member).
4. Write `terms.md` and `privacy.md` (start from `hub/config/`), set `--contact`, and
   `--name`/`--about` for the landing page.
5. Monitor `/health`, back up the data folder daily, and watch disk: each member can fill
   `--quota` + `--share-quota`.
6. Billing is outside the server: there are no payment hooks; you make and revoke invites
   and ban accounts by hand (or script the admin panel's forms).
