# CryptoUpload

**Your files, encrypted on your device before they reach any cloud.**

CryptoUpload turns any storage you already have — Google Drive, an S3 bucket, a NAS, a WebDAV
server, an external disk — into a private vault. Files are encrypted on your computer; the
provider only ever stores ciphertext. There is no CryptoUpload server, no account to create and
no telemetry.

Free and open source (GPLv3) · Windows and Linux desktop app ·
[cryptoupload.org](https://cryptoupload.org)

> **CryptoUpload has not yet had an independent security audit.** Do not rely on it as the only
> protection for information whose disclosure would seriously harm you. See
> [SECURITY.md](SECURITY.md).

## What you get

- **Zero-knowledge by construction.** One password and a 24-word recovery phrase. Keys never
  leave your device; names and contents are both encrypted.
- **Your storage, your choice.** S3 and compatibles (AWS, MinIO, Backblaze B2, Wasabi), SFTP,
  WebDAV (Nextcloud, ownCloud), Google Drive, Dropbox, OneDrive, or a plain local folder.
  [How to connect each one](docs/PROVIDERS.md).
- **Automatic backup and manual upload.** Keep folders in sync on a schedule, or drop in a
  single file. Scheduled syncs keep running from the notification area after you close the
  window. Interrupted uploads resume instead of starting again.
- **A drive you can open.** Mount the vault as a normal disk and work on your documents
  directly; files are decrypted on the fly and streamed, even from a cloud-only vault.
- **History and undo.** Versions of every synced file, restore to a point in time, a recycle
  bin, and conflict handling that keeps both copies rather than guessing.
- **Built to resist ransomware and tampering.** Anomaly detection before a sync propagates mass
  changes, immutable backups through S3 Object Lock, and per-file anti-rollback protection that
  refuses an old copy presented as current.
- **Extra protection when you need it.** Optional hardware key (FIDO2) as a true second factor,
  recovery phrase split into shares, decoy files, a private vault profile that hides file sizes
  and counts from the provider, network proxy support, printable recovery kit. All off by
  default; each one is explained in the app with what it costs.
- **Sharing without a server.** Add another person to a vault with public-key cryptography
  (X25519); removing a member really rotates the keys.

## Why you can check it instead of trusting it

- **Open source.** All the code is here, under the GPLv3.
- **No home-made cryptography.** The vault uses the open
  [Unified Vault Format](https://github.com/encryption-alliance/unified-vault-format) (AES-256-GCM
  for contents, AES-SIV for names, Argon2id for the password).
- **No telemetry, enforced.** A check in the project's CI fails if any component other than the
  storage backends and the updater gains a network dependency (`cargo run -p xtask -- check-no-http`).
- **Stated limits.** CryptoUpload has not had an independent security audit; see
  [SECURITY.md](SECURITY.md).

## Get it

Download the installer for your system from the
[Releases page](https://github.com/cryptoupload/CryptoUpload/releases) and check its SHA-256
against the one GitHub shows next to the file, or against `SHA256SUMS.txt` in the same release.
Each installer is also signed with the project's key: [how to check it](SECURITY.md#check-a-download).
Builds published anywhere else, forks included, are not signed by the project.

| System | Package |
|---|---|
| Windows 10/11 | `.exe` installer (per-user, no administrator rights) |
| Linux | `.deb` (Debian/Ubuntu) or `.AppImage` |

The beta is not code-signed: Windows SmartScreen will ask for confirmation on first launch. Updates are verified with a signature before being installed.
The virtual drive needs [WinFsp](https://winfsp.dev) on Windows and FUSE 3 on Linux.

## Five-minute start

1. Open CryptoUpload; a guided first run explains each step.
2. Create a vault in a local folder or on one of your storage providers, and choose a password.
3. Write the 24-word recovery phrase on paper and confirm it.
4. Drop files into the vault, or set up a folder to be backed up automatically.

## Build from source

```bash
git clone https://github.com/cryptoupload/CryptoUpload.git
cd CryptoUpload/app
cd apps/desktop && npm ci && npx tauri build   # desktop app
```

Rust stable and Node.js are required. On Linux the build also needs these system packages:
`libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev libsoup-3.0-dev libgtk-3-dev librsvg2-dev
libayatana-appindicator3-dev pkg-config libudev-dev`.

## Known limitations

- **No independent security audit yet** (see above).
- **Dropbox and OneDrive are experimental**: tested against simulated servers, not yet with real
  accounts. Use them for data you also keep elsewhere.
- **Backblaze B2 and Wasabi**: CryptoUpload checks, before saving the remote, that the provider
  refuses to overwrite a file two devices create at the same instant, and refuses the remote with
  an explanation if it does not. The check has not yet run against a real account on these two.
- **Hardware keys (FIDO2)** are tested against a software authenticator; specific physical keys
  may behave differently. Keep your password and recovery phrase.
- **On Windows**, while a vault is unlocked, another program running under your user account may
  be able to read its keys from memory. On Linux the app blocks this (an administrator still can).
- **Scheduled syncs need the program running** (window closed is fine, quitting is not). With the
  virtual drive mounted, closing the window quits the program. On GNOME the notification-area
  icon needs the AppIndicator extension.
- **Linux virtual drive**: files are never executable; copy a program out of the drive to run it.
- **Conflict copies carry the computer's name** (the name set in the system, for example
  `report (conflict LAPTOP 2030-01-15 09.15.00 UTC).pdf`). The name stays inside the encrypted vault: the storage
  provider never sees it, but the other members of a shared vault do.
- **Translations**: English is the reference; the other languages are drafts awaiting review by
  a native speaker, and the app says so.

## Status

Public beta. Vault format, sync engine, seven storage backends, desktop app, virtual drive and
installers are in place. Mobile apps are planned. What changed in each release:
[CHANGELOG.md](CHANGELOG.md).

## Contributing and security

Contributions are welcome under the DCO (sign-off, no CLA) —
[CONTRIBUTING.md](CONTRIBUTING.md). Report vulnerabilities privately to
**security@cryptoupload.org**, never in a public issue
([SECURITY.md](SECURITY.md)). For everything else: **info@cryptoupload.org**.
Found a bug or have an idea? The app has a **Report a problem** button in the side bar: it prepares
the message and you choose how to send it (email, GitHub issue or copy).

## Licence

[GPLv3](LICENSE) with an
[additional permission for app stores](LICENSE-EXCEPTION.txt).
Copyright © 2026 CryptoUpload.
