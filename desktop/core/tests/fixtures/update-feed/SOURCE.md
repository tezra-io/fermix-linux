# The update feed fixture

Both sides of the update feed are tested against these files: the release's writer, by
`desktop/packaging/update_feed_test.sh`, and the app's reader, by core's tests.
`desktop/packaging/update_feed_fixture.sh <dir>` writes them with `update_feed.py`, the way a release
writes its feed: each release is an item, merged into the feed before it and signed.
`update_feed_test.sh` writes them again and expects the same bytes.

- `test-key-linux-feed-test.seed` and `test-key-linux-feed-test.pem`: the test key `linux-feed-test`.
  Its 32-byte seed is SHA-256 of the ASCII text `linux-feed-test`. It is a test key: it signs nothing
  but fixtures, and `desktop/packaging/update-keys.json`, the keys the app trusts, never lists it.
- `test-keys.json`: its public key, in the form of `update-keys.json`.
- `appcast.json`: the signed feed.
- `expected-decode.json`: what `update_feed.py verify --decoded` reads from it, the key id and the
  payload.

| Release | Engine | Critical | Minimum installed | Notes |
|---------|--------|----------|-------------------|-------|
| 0.5.0   | 0.18.0 | no       | 0.4.0             | 3, with non-ASCII text |
| 0.4.0   | 0.17.0 | yes      | none              | 1 |
| 0.3.0   | 0.16.0 | no       | none              | 2, one with a `code` span |

The feed was generated at 2026-11-20T18:05:00Z. Each release's packages are under
`https://github.com/tezra-io/fermix-linux/releases/download/v<version>/`. Each package's bytes are
the ASCII text `test package <file>` and a newline, so a test can serve them and match their size and
sha256. Each engine's source commit is SHA-1 of `engine <version>`.
