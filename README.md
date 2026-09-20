# composelint

A linter for the misconfigurations that keep showing up in real
`docker-compose.yml` files: floating image tags, containers with no
restart policy, `network_mode: host`, `privileged: true`, and published
ports bound to every interface on the host instead of just loopback.
There's no dominant standalone tool for this the way `hadolint` covers
Dockerfiles — most of it lives as tribal knowledge or a checklist in
someone's head. This is a static binary that checks it automatically.

## Usage

```bash
composelint path/to/docker-compose.yml              # human-readable findings, exits 1 if any
composelint path/to/docker-compose.yml --json        # machine-readable findings
```

Exit code `0` means clean, `1` means at least one finding, `2` means the
file couldn't be read or wasn't valid Compose YAML.

## Rules

- **`unpinned-image`** — `image: nginx` (no tag) or `image:
  nginx:latest` (explicit but equally unreproducible). A digest-pinned
  image (`@sha256:...`) or one using a `$VARIABLE` this tool can't
  resolve statically is left alone; a service with no `image:` field at
  all (a `build:`-only service) isn't checked.
- **`no-restart-policy`** — no `restart` key set on the service. Any
  explicit value, including `restart: "no"`, is treated as a deliberate
  decision and not flagged — only the key's total absence is.
- **`host-network-mode`** — `network_mode: host`.
- **`privileged-container`** — `privileged: true`. This is the one rule
  reported at `error` severity rather than `warning`, since it's rarely
  something anyone means to leave in place.
- **`unbound-published-port`** — a `ports:` entry with a fixed host port
  and no host-IP restriction (`"5432:5432"`, or `"0.0.0.0:5432:5432"`
  spelled out explicitly), in both the short string form and the long
  mapping form (`target`/`published`/`host_ip`). The bare single-number
  short form (`"5432"`, no colon) is a materially different, lower-risk
  shape — Compose picks a random ephemeral host port for it rather than
  a developer-chosen fixed one — and isn't flagged.

## Status: built, 30 unit tests passing, verified against fixtures and a real archived compose stack — including a real bug the archived file caught

- **30 unit tests** (`cargo test --lib`): YAML parsing tolerant of a
  real-world file using fields this tool doesn't inspect (a strict
  schema would reject it; a generic `Value` walk doesn't), both port
  syntaxes in both the "has a host-IP" and "doesn't" shape, and every
  rule individually against small hand-built `Service` values including
  each rule's true-negative (an explicit `restart: "no"`, a
  digest-pinned image, `network_mode: bridge`, a loopback-bound port, a
  bare ephemeral port). Two fixture-level tests run the full pipeline
  against `fixtures/bad-compose.yml` (two services constructed to hit
  all five rules) and `fixtures/good-compose.yml` (three services meant
  to pass every rule cleanly, including one using the bare-port short
  form specifically to confirm it doesn't false-positive) and assert
  the exact rule sets each produces.
- **`cargo clippy --all-targets -- -D warnings`**: clean.
- **Found and fixed a real bug by running this against a real compose
  file, not just its own fixtures**: this monorepo's own (now-archived)
  `_archive/SeekersConnect/docker-compose.yml` was pulled from git
  history (`git show HEAD:_archive/SeekersConnect/docker-compose.yml`)
  and run through the tool as a live sanity check. Every one of that
  file's five published ports uses the pattern
  `"${RUSTFS_API_PORT:-9000}:9000"` — an env var with a bash-style
  default. The first version of the port parser did a plain
  `str::split(':')`, which misread the `:-` inside the `${...}` braces
  as a field separator and treated `${RUSTFS_API_PORT` as a literal
  host-IP restriction, silently passing all five ports as "bound" when
  none of them actually specify a host IP at all — a real false
  negative on the exact rule this tool exists to enforce, not a
  hypothetical one. Fixed by tracking brace depth and only splitting on
  a `:` at depth zero (`split_port_string` in `src/parser.rs`);
  reran against the same real file afterward and got the correct 5
  `unbound-published-port` findings (one per service) plus the
  pre-existing `rustfs/rustfs:latest` `unpinned-image` finding — 6 real,
  correct findings against a real multi-service production-shaped
  compose file, none of them false positives on inspection. Regression
  tests for both the bare env-var-default case and a host-IP combined
  with an env-var-default port are in `src/parser.rs`.

**Not done / deliberately deferred**: `deploy.restart_policy` (the
Swarm-mode equivalent of `restart`) isn't recognized as satisfying the
restart-policy rule, so a Swarm-targeted compose file using only that
field gets a false positive; `.env`-file variable resolution — a
`${VAR}` in an image tag or port is treated as "unknown, don't flag"
rather than actually substituted from a real `.env` file, so a genuinely
floating tag hidden behind a variable slips through; and no
`--ignore <rule>` flag to suppress a specific rule.
