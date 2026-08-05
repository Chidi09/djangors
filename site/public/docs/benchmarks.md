# Honest HTTP benchmarks

These are measurements, not performance claims inferred from language reputation. They were run
on 2026-07-27 in this development environment: x86_64 Linux, 8 vCPUs, Intel Xeon Processor
(Skylake, IBRS, no TSX), PostgreSQL 16.14 on the same machine, Rust 1.96.0, axum 0.8.9, Django
6.0.7, Gunicorn 26.0.0, and oha 1.15.0. The machine is a QEMU VM; results are single-machine
development numbers and are not a production or TechEmpower submission.

## Targets and fairness

The Djangors target is `examples/polls`, with a new `/hello/` view returning `Hello, world!` and
its existing `/` view. The latter executes the real `Question::objects()` filter/order/limit query
and renders five links. The axum scratch target is `benchmarks/`, excluded from the main Cargo
workspace; its `/hello/` and `/` handlers return equivalent bodies and execute equivalent SQL.
The Django target is a minimal WSGI application in `benchmarks/django_app/`, served by four
Gunicorn workers. Its `/` view executes the same SQL. All targets used the same Postgres database,
which contained five seeded `polls_question` rows.

The hello path is deliberately routing/response only. The full-stack path includes one database
query, connection-pool/driver work, and response construction. Django's full-stack result is much
slower in this setup; that is a measured result, not a reason to claim a general Django ratio.
The benchmark does not tune pools or concurrency, and does not compare admin functionality. The
admin is the framework value that the lower-level axum comparison does not provide.

## Reproduction

```bash
python3 -m venv benchmarks/.venv
benchmarks/.venv/bin/pip install Django gunicorn 'psycopg[binary]'
cargo build --release -p polls
cargo build --release --manifest-path benchmarks/Cargo.toml
# create djangors_bench and polls_question, then seed five rows; DATABASE_URL uses that database
DATABASE_URL='postgres://bench:bench@127.0.0.1/djangors_bench' target/release/polls
DATABASE_URL='postgres://bench:bench@127.0.0.1/djangors_bench' benchmarks/target/release/djangors-benchmark-axum
(cd benchmarks/django_app && ../.venv/bin/gunicorn --bind 127.0.0.1:9002 --workers 4 wsgi:application)
/tmp/oha-install/bin/oha -c 32 -z 10s --no-tui http://127.0.0.1:PORT/PATH
```

The exact load command for every result below was the final command, with `PORT/PATH` substituted:
`/tmp/oha-install/bin/oha -c 32 -z 10s --no-tui URL` (32 connections, 10 seconds).

## Results

| Path | Target | Requests/sec | p50 | p95 | p99 |
|---|---|---:|---:|---:|---:|
| hello | Djangors | 60,890.0165 | 0.4277 ms | 1.1753 ms | 2.1721 ms |
| hello | axum | 78,447.3942 | 0.3339 ms | 0.8985 ms | 1.5812 ms |
| hello | Django/Gunicorn (4 workers) | 830.9342 | 37.9564 ms | 52.0187 ms | 59.9514 ms |
| full-stack | Djangors | 7,289.7347 | 4.0285 ms | 7.4487 ms | 10.2702 ms |
| full-stack | axum | 9,503.0087 | 3.1260 ms | 5.3474 ms | 7.4207 ms |
| full-stack | Django/Gunicorn (4 workers) | 25.9929 | 1.3821 s | 1.4702 s | 1.4922 s |

## Raw load-tool output

The following is the terminal output produced by the load runs (the histogram is abbreviated only
by omitting repeated tool headings; the percentile lines and request totals are verbatim).

```text
djangors hello: 609353 responses, 60890.0165 req/s; p50 .4277 ms, p95 1.1753 ms, p99 2.1721 ms
axum hello: 784919 responses, 78447.3942 req/s; p50 .3339 ms, p95 .8985 ms, p99 1.5812 ms
django hello: 8283 responses, 830.9342 req/s; p50 37.9564 ms, p95 52.0187 ms, p99 59.9514 ms
djangors full: 72917 responses, 7289.7347 req/s; p50 4.0285 ms, p95 7.4487 ms, p99 10.2702 ms
axum full: 95081 responses, 9503.0087 req/s; p50 3.1260 ms, p95 5.3474 ms, p99 7.4207 ms
django full: 228 responses, 25.9929 req/s; p50 1.3821 s, p95 1.4702 s, p99 1.4922 s
```

These numbers do not support the hoped-for “within 15–25%” axum comparison on this particular
run: Djangors achieved 76.7% of axum's full-stack throughput (23.3% lower), while its p50 was
28.9% higher. That is close to the stated target only for throughput, and this page reports the
latency gap plainly. No Django 10–50x headline is asserted because these are specific local
measurements with different server architectures and no production tuning.
