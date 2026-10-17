# Vendored copy: what is changed

A copy of the `src/`, `Cargo.toml`, `LICENSE` and `README.md` of
[uglyoldbob/webpki](https://github.com/uglyoldbob/webpki) branch `main5`, commit `94030fb`
("Add logging and try to parse v1 certs."), itself a fork of rustls-webpki 0.103.9 (ISC licence).

The fork accepts the X.509 **version 1** root certificate that Android Auto phones present. Stock
rustls-webpki rejects it with `UnsupportedCertVersion`, and the handshake fails.

Change against that fork: the debug `log::error!` lines in `src/cert.rs` ("Checkin 1" and so on) are now
`log::trace!`.

It is wired in through `[patch.crates-io]` in the workspace `Cargo.toml`. Check that it is really used with
`cargo tree -i rustls-webpki`: it must print a `vendor/rustls-webpki` path. If cargo picks another version,
the patch does nothing and the handshake fails.
