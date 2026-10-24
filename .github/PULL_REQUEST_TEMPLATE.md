## What changes and why

## Checklist

- [ ] Every commit has `Signed-off-by` (`git commit -s`) — DCO, not a CLA
- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` and
      `cargo deny check` pass locally
- [ ] If this touches the vault format (`cu-crypto`/`cu-format`): it was discussed in an issue first
- [ ] If this touches `cu-format`: truncation is still detected at every chunk boundary
- [ ] No key, token or password is logged or printed, not even in debug messages
- [ ] User-visible changes are described in `CHANGELOG.md`
