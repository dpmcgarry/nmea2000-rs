# canboatjs test fixtures

`pgns.json` is converted from the per-PGN decoder tests in
[canboatjs](https://github.com/canboat/canboatjs) (`test/pgns/*.js`),
Copyright canboatjs contributors, licensed under the Apache License 2.0.
The upstream commit is recorded in `UPSTREAM_VERSION`.

Refresh with `./scripts/update-canboatjs-tests.sh`.

`known-failures.txt` lists the field checks that do not match yet. The
`canboatjs` test fails on any new mismatch, and also when a listed check
starts passing, so the list stays accurate. Rewrite it with:

```
CANBOATJS_BLESS=1 cargo test -p nmea2000-pgns --test canboatjs
```

Print the full per-field report with:

```
CANBOATJS_REPORT=1 cargo test -p nmea2000-pgns --test canboatjs -- --nocapture
```
