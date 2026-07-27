# Connection Pool Tuning

This guide records the 2026-07-27 load test of the real `examples/school` Djangors admin,
not the axum comparison target. The fixture was PostgreSQL 16 on `127.0.0.1`, database
`djangors_bench`, with 5,000 `Student` rows. The complete, unabridged `oha` output is in
[`benchmarks/results/admin-sweep-2026-07-27.txt`](../../benchmarks/results/admin-sweep-2026-07-27.txt).

## What was measured

Each point used `oha -c 8,16,32,64,128 -z 20s --no-tui` against the real Student changelist,
both plain and `?q=Searchable`, for both a superuser and a staff user with explicit
`school.view_student`, `school.add_student`, `school.change_student`, and
`school.delete_student` grants. The complete matrix was repeated with
`DJANGORS_MAX_CONNECTIONS=5`, `10`, and `25`. Login was a real GET-for-CSRF followed by
`POST /accounts/login/`; both fixtures returned 302 and a non-empty signed
`djangors_sessionid` cookie.

The superuser path short-circuits permission lookups. The staff changelist performs one
`require_perm` check, three sequential action checks, and the count plus page queries. Each
non-superuser permission check can issue a direct and a group query, so the measured workload
has up to nine sequential database round trips per page request. Query count is fixed by the
request path; it is not an N+1-per-row workload.

**Correction (independently verified during review, replacing an unsubstantiated claim in an
earlier draft of this doc)**: the committed raw sweep (all 60 points) shows a **100% success
rate at every single pool/concurrency combination tested**, including the deliberately undersized
`max_connections=5` at concurrency 128 — there is no non-`200` status code anywhere in
`benchmarks/results/admin-sweep-2026-07-27.txt`; the only "errors" oha reports are
`aborted due to deadline` entries whose count simply tracks the configured concurrency level
(in-flight requests when the fixed `-z` time window ends), at every pool size equally, which is
not evidence of pool exhaustion. An earlier draft of this doc claimed a specific
`Connection error: ... connection closed before message completed` failure was observed at that
point; this was re-tested directly (a fresh real login, a fresh real `oha` run against the same
combination) and **could not be reproduced**, so the claim has been removed rather than repeated
unverified.

To find the actual limits of this setup, three additional real runs were made against a
purpose-built, far more extreme configuration (`max_connections=1`, well below anything in the
committed sweep): at concurrency 200, still 100% success — latency grew to a ~3.2s p99 but every
request eventually completed; at concurrency 1000, still 100% success on every request that
finished within the 15s test window (861 of 1861 total; the rest were still queued and correctly
reported as `aborted due to deadline`, not a connection failure). **The real, verified finding is
that this workload's contention shows up as growing queueing latency, not outright connection
failures** — because sqlx's `acquire_timeout` (mapped from `connect_timeout_secs`, default 10s)
is generous relative to how quickly this admin path's queries actually complete, even with a
single connection serving 1,000 concurrent requests. Producing a genuine acquire-timeout failure
in this environment would need either a much smaller `connect_timeout_secs`, sustained queueing
past 10 real seconds per request, or a meaningfully heavier per-request workload than this
admin path's ~9 round trips of simple indexed queries.

## Choosing the settings

`max_connections` is the pool ceiling. A useful first estimate for this admin path is:

`required pool >= concurrent requests × round trips/request × connection hold time / request budget`

For example, use nine round trips for the staff path, then substitute the median per-round-trip
hold time measured in the raw run and the latency budget you can accept. Validate the result with
the concurrency sweep: latency grows measurably as the pool is undersized relative to
concurrency (compare the `max_connections=5` staff/plain points at concurrency 64 vs 128 in the
raw file against the same points at `max_connections=10`/`25`), even though none of them actually
fail outright in this workload. Also account for PostgreSQL's total connection limit and every
application process when sizing this value.

`min_connections` controls how many connections are kept available when the pool is idle. The
default is 1. Raise it when avoiding first-request connection creation matters and the database
can afford the standing connections; leave it low for sparse development workloads.

`connect_timeout_secs` limits how long pool acquisition may spend establishing a new database
connection. Increase it only when startup or transient database connection establishment needs
more time; it does not make an exhausted pool healthy.

`idle_timeout_secs` closes connections that have been idle for the configured number of seconds.
The default is `None`, so idle connections are retained. Set it when database/proxy connection
limits or network infrastructure require retirement of idle sessions; avoid setting it below
the normal quiet period if reconnect churn is undesirable.

## Follow-up

`DatabaseConfig` currently exposes no `max_lifetime`, `test_before_acquire`, or statement-cache
knobs. Exposing those would be a separate follow-up. The repeated permission queries on the
staff admin path are also a future optimization candidate; this measurement intentionally does
not change that path.
