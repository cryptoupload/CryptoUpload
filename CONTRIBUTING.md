# Contributing to CryptoUpload

CryptoUpload is in public beta and under active development. Before opening a substantial pull
request, open an issue to discuss it — especially if it touches `cu-crypto` or `cu-format`, the
crates that need the most careful review. A proposal that goes against an established design
decision needs a new, concrete reason.

## DCO, not a CLA

The project uses the **Developer Certificate of Origin**. You do not transfer any right: you
only certify that you are allowed to contribute your code under the project licence.

Every commit needs a `Signed-off-by` line with a name and an email address you can be reached
at. Git adds it for you:

```bash
git commit -s -m "your message"
```

By signing off you state the following (DCO 1.1, full text):

```
Developer Certificate of Origin
Version 1.1

Copyright (C) 2004, 2006 The Linux Foundation and its contributors.
1 Letterman Drive
Suite D4700
San Francisco, CA, 94129

Everyone is permitted to copy and distribute verbatim copies of this
license document, but changing it is not allowed.


Developer's Certificate of Origin 1.1

By making a contribution to this project, I certify that:

(a) The contribution was created in whole or in part by me and I
    have the right to submit it under the open source license
    indicated in the file; or

(b) The contribution is based upon previous work that, to the best
    of my knowledge, is covered under an appropriate open source
    license and I have the right under that license to submit that
    work with modifications, whether created in whole or in part
    by me, under the same open source license (unless I am
    permitted to submit under a different license), as indicated
    in the file; or

(c) The contribution was provided directly to me by some other
    person who certified (a), (b) or (c) and I have not modified
    it.

(d) I understand and agree that this project and the contribution
    are public and that a record of the contribution (including all
    personal information I submit with it, including my sign-off) is
    maintained indefinitely and may be redistributed consistent with
    this project or the open source license(s) involved.
```

A pull request with a single commit missing `Signed-off-by` does not pass the `dco` CI job.

## Licence header

Every `.rs` file starts with:

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
```

## Before you open a pull request

The CI runs the checks below and all of them must pass. Run them from `app/` (the desktop check from
`app/apps/desktop/`):

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
cargo deny check
cargo run -p xtask -- check-layering
cargo run -p xtask -- check-no-http
cargo run -p xtask -- check-error-codes
cargo run -p xtask -- check-windows   # needs the x86_64-pc-windows-gnu target and MinGW
(cd apps/desktop && npm ci && npm run check)
```

The CI also builds the workspace with the oldest supported Rust (1.89) and, on a pull request,
checks that every commit has its `Signed-off-by` line.

## Translations

The desktop app speaks the languages listed in `app/apps/desktop/src/lib/i18n/codes.json`. English
is the reference; every other language is one file in `app/apps/desktop/src/lib/i18n/locales/`
with exactly the same keys. A language is offered in the app as soon as its file exists.

**Fixing a text does not need any code.** In the app, next to the language menu, open *Help
translate*: you see each English text with a box for your language, you can try your edits live
in the app, and then save a file or write an email to `translations@cryptoupload.org`. Nothing is
sent by the app itself.

For maintainers, a received file is applied like this (it is treated as data: only its keys and
texts are used, and a text that changed in the meantime is skipped):

```bash
cd app/apps/desktop
node tools/apply-translation.mjs cryptoupload-translation-de.json           # shows the changes
node tools/apply-translation.mjs cryptoupload-translation-de.json --write   # applies them
```

Rules for a translation, checked by `npm run check`:

- Placeholders in curly brackets (`{name}`) and marks in angle brackets (`<b>…</b>`) stay exactly
  as in English. So do product and protocol names (CryptoUpload, S3, SFTP, WebDAV, FIDO2, …).
- A text with a number is a set of plural forms (`one`, `few`, `many`, `other`, …): write the
  forms your language uses. `other` is always required.
- No text is written in the code: `node tools/check-hardcoded.mjs` lists any that slipped in.
- Keep the same word for the same thing everywhere: *vault*, *remote* (a configured storage
  location), *job* (a folder kept in sync), *run* (one execution of a job), *recovery phrase*,
  *share* (one piece of a split recovery phrase), *mailbox* (the one-way drop), *decoy file*.
- English is the reference; every other language is marked as a draft in the app until a native
  speaker has reviewed it.

Texts about passwords, the recovery phrase and anything that deletes data deserve the most care:
a wrong word there can cost someone their files.

## Project rules

These hold for every contribution.

1. **No invented cryptography.** The format is UVF. When in doubt, stop and ask in an issue
   instead of improvising a variant.
2. **The end-of-file block rule is not optional.** It is the only defence against truncation.
   A change to `cu-format` must never weaken truncation detection at any chunk boundary.
3. **`cu-crypto` and `cu-format` do no I/O, no async, and take no dependency outside the
   allowlist.** A change that adds an external dependency to either is almost certainly wrong.
4. **Never log or print a key, a token or a password**, not even in debug or error messages.
5. **The sync engine never loses data silently.** Between the risk of losing something and the
   risk of leaving the user with an extra file, always choose the second (`keep-both`). A
   destructive or irreversible option shows its consequences before it is enabled, not after.
6. **No plain HTTP without an explicit opt-in and a visible warning.**
7. **The format is shared.** It is the Unified Vault Format; propose a change in an issue first,
   never inside an unrelated pull request.
8. **Claims need evidence.** Do not add a statement about security, speed or compatibility to the
   documentation unless you can show how it was verified.

## Code of conduct

By taking part you accept the [Code of Conduct](CODE_OF_CONDUCT.md) (Contributor Covenant 2.1).
Questions: info@cryptoupload.org.
