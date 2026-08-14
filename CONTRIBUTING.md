# Contributing

DonutHLE is an early research prototype. Keep changes small, focused, and covered by tests where practical.

Do not add proprietary APKs, system images, SDK archives, or generated build output. Compatibility fixtures must be original or redistributable under a clear license.

Before opening a pull request:

```sh
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```
