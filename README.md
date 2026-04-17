# ⛩️ sh3rine

Stateless async S3 static site proxy. Point it at a Ceph RGW (or any S3-compatible) endpoint, map hostnames to buckets, and it handles all the edge cases that raw S3 path-style access gets wrong.

## Why

S3's native static website hosting requires a separate endpoint per bucket and doesn't exist in self-hosted Ceph without significant extra setup. Doing the routing in a reverse proxy (Caddy rewrites, nginx try_files) is fragile and hard to reason about. sh3rine is a small dedicated server that does exactly one thing cleanly.

## Features

- **Smart path resolution** — extensionless URLs try `.html` → `index.html` → exact path, in that order
- **Custom 404 pages** — serves `404.html` from the bucket root before falling back to a plain 404
- **Correct Content-Type** — overrides S3's `application/octet-stream` using file extension, with byte sniffing fallback (handles extensionless AVIF, WebP, etc.)
- **LRU cache** — small files cached in memory with TTL; large files always streamed
- **Dynamic host → bucket mapping** — exact matches and regex patterns with named capture groups
- **Strips S3 noise** — drops `content-encoding: aws-chunked` and other S3-internal headers

## Configuration

All config via environment variables.

| Variable | Default | Description |
|---|---|---|
| `ENDPOINT` | `http://localhost:9000` | S3/RGW base URL |
| `LISTEN` | `0.0.0.0:8080` | Bind address |
| `HOSTS` | — | Exact hostname→bucket mappings, `\|`-separated: `host:bucket\|host2:bucket2` |
| `HOST_PATTERNS` | — | Regex patterns with named groups, `\|`-separated: `pattern:template\|...` |
| `CACHE_MAX_MB` | `128` | Total cache size cap in MB |
| `CACHE_TTL_SECS` | `60` | Cache entry TTL |
| `CACHE_MAX_FILE_KB` | `512` | Files larger than this are streamed, not cached |
| `S3_TIMEOUT_SECS` | `15` | Per-request timeout for S3 fetches |
| `CACHE_CONTROL` | — | Value for `Cache-Control` response header (e.g. `public, max-age=300`) |
| `RUST_LOG` | — | Log level (`info`, `debug`, etc.) |

### Host mapping examples

**Exact:**
```
HOSTS=jmarya.me:jmarya-me|docs.example.com:my-docs-bucket
```

**Regex with named capture groups** (bucket name extracted from hostname):
```
HOST_PATTERNS=s3pub-(?P<bucket>[^.]+)\.hydrar\.de:{bucket}
```

Both are checked in order — exact matches first, then patterns top to bottom.

## Path resolution

For a request to `/some/page`:

| Path type | Candidates tried (in order) |
|---|---|
| `/` or trailing slash | `index.html` |
| Has extension (`.jpg`, `.html`, …) | exact path |
| Extensionless | `some/page.html` → `some/page/index.html` → `some/page` |

On total miss, serves `404.html` from the bucket root with a 404 status. If that's also missing, returns a plain-text 404.

## Building

```sh
# Binary
nix build

# Container image (loads directly into Docker)
nix build .#dockerImage && docker load < result
```

## Running

```sh
ENDPOINT=https://s3.example.com \
HOSTS=jmarya.me:jmarya-me \
HOST_PATTERNS='s3pub-(?P<bucket>[^.]+)\.hydrar\.de:{bucket}' \
RUST_LOG=info \
./sh3rine
```

## Response headers

| Header | Values | Meaning |
|---|---|---|
| `x-sh3rine-cache` | `HIT` / `MISS` / `STREAM` | Whether the response came from cache |
