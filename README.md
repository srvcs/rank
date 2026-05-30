# srvcs-rank

A comparison service of the srvcs.cloud distributed standard library.

Its single concern: **the 1-indexed rank of a value within a list.** It does no
comparison of its own. For each element `v` of `values` it asks
[`srvcs-lessthan`](https://github.com/srvcs/lessthan) whether `v < value`,
counting how many elements are strictly less:

```text
count = 0
for v in values:
    if lessthan(v, value):   # one HTTP call to srvcs-lessthan per element
        count += 1
result = count + 1
```

So `rank([10,20,30], 20) == 2` and `rank([10,20,30], 5) == 1`. The rank of a
value against the **empty list** is `1`, and makes no dependency calls at all.

This is an orchestrator: it never calls `srvcs-isnumber` directly. Input
validation propagates from its dependency — if `srvcs-lessthan` rejects an
operand, that `422` is forwarded verbatim.

## API

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/` | Service identity, concern, and dependency list |
| `POST` | `/` | Rank `value` within `values` |
| `GET` | `/healthz` `/readyz` `/metrics` `/openapi.json` | srvcs service standard surface |

```sh
curl -s -X POST localhost:8080/ -H 'content-type: application/json' -d '{"values": [10, 20, 30], "value": 20}'
# {"values":[10,20,30],"value":20,"result":2}
```

Responses:

- `200 {"values": [...], "value": v, "result": n}` — evaluated.
- `422` — an operand is not a valid integer, forwarded from `srvcs-lessthan`.
- `500` — `srvcs-lessthan` returned an unusable response.
- `503` — the `srvcs-lessthan` dependency is unavailable.

## Dependencies

- [`srvcs-lessthan`](https://github.com/srvcs/lessthan)

A single request fans out across the dependency graph: one `rank → lessthan`
call per list element, and each `lessthan` in turn validates both operands via
`lessthan → isnumber`.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `SRVCS_BIND_ADDR` | `0.0.0.0:8080` | Bind address |
| `SRVCS_LESSTHAN_URL` | `http://127.0.0.1:8081` | Base URL of `srvcs-lessthan` |
| `SRVCS_ENV` | `development` | Environment label for logs |
| `RUST_LOG` | `info,tower_http=info` | Tracing filter |

## Local checks

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Orchestration tests stand up a mock `srvcs-lessthan` in-process that **actually
computes** `a < b` from the request body, so the composition is genuinely
exercised (e.g. `rank([10,20,30], 20) == 2`). See
[`srvcs/platform`](https://github.com/srvcs/platform) for the shared standard.

> Note: the `cargoHash` in `flake.nix` is inherited from the template and must be
> refreshed with a `nix build` before the Nix gates pass.
