# Changelog

All changes that matter to users are recorded here.
Format: [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) — versioning:
[Semantic Versioning](https://semver.org/).

## [0.1.0-beta.2]

### Added

- **Licence texts installed with the app**: the GPLv3, its app store exception and the licences
  of the third-party components (fonts and libraries) are installed in a `licenses` folder next
  to the program.
- **Check a download yourself**: [SECURITY.md](SECURITY.md#check-a-download) explains how to
  verify the signature of an installer with minisign.

### Changed

- **Release signing**: the key that signs updates is used only in a separate, isolated step of the
  release process, after the installers are built; every signature is checked against the app's
  public key before the release is published.
- **Recovery kit preview** is shown in an isolated frame that cannot run scripts.
- The app identifier is now `org.cryptoupload.desktop`, matching the project's domain.
- Stricter security policy for the app window (no plugins, no form submissions, no embedding).

### Security

- Dependency updates for published advisories: tokio 1.53.3, rustls-webpki 0.103.16,
  webpki-roots 1.0.11 (distrusted emSign certificates), crypto-bigint 0.7.6 and ctutils 0.4.5
  (constant-time fixes).

### Fixed

- **Linux, recent distributions (Ubuntu 26.04 and similar):** the `.deb` did not start, because
  it needed an older FUSE library (`libfuse3.so.3`) that these systems no longer ship; the
  virtual drive now only needs the `fuse3` package. The `.AppImage` opened an empty window,
  because the Wayland libraries packed inside it did not match the system's graphics drivers;
  it now uses the system's own.
- Closing the app right after Google Drive or OneDrive renewed its access could lose the renewed
  access, so the remote had to be signed in again.
- Italian: clearer texts about the proxy setting, key rotation and the recovery phrase.

## [0.1.0-beta.1]

A complete first version for the main use: encrypt on your device, back up to the storage you
choose, work from a desktop app for Windows and Linux. It has not yet had an independent security
audit — see [SECURITY.md](SECURITY.md).

### Vault and encryption

- **Open vault format (UVF)**: AES-256-GCM in 32 KiB chunks for contents, AES-SIV-512 for names,
  Argon2id for the password. One file is one encrypted object.
- **Recovery phrase**: 24 words, independent of the password. Changing the password does not
  re-encrypt your data.
- **Anti-rollback protection**: a storage provider serving an old copy of a file, or of the vault
  configuration, is detected and refused until you confirm that you restored a backup yourself.
  If the registry itself becomes unreadable (disk fault, a program altering files in the vault),
  Settings has a “Restart the registry” button with a warning and a confirmation.
- **Private vault profile**, chosen when you create the vault: the provider sees only size
  and name-length bands; optional decoy files round the number of files per folder. Padding costs
  at most about 12 % extra space (small files pay the most; about 2 % on a mixed set of files
  where large ones dominate); decoys add about 10 % more on that same set, and almost double in
  the worst case of folders with a few small files.
- New passwords must be at least 8 characters.

### Storage

- **Seven backends**: local folder, S3 and compatibles (with Backblaze B2 and Wasabi presets),
  SFTP with mandatory host-key verification, WebDAV, Google Drive (scope `drive.file` only),
  Dropbox (app folder only) and OneDrive. Dropbox and OneDrive are experimental.
- **Resumable uploads** that survive the process being killed.
- **Immutable backups** with S3 Object Lock.
- **Per-remote passwords**: each saved remote has its own password, so people sharing a computer
  do not share credentials.
- **Network proxy** (SOCKS5/HTTP) for the whole program. If the proxy does not answer the network
  stops; it never falls back to a direct connection.

### Sync and backup

- **Folder ↔ vault sync** with three-way diff, an intent journal and keep-both on conflicts:
  nothing is overwritten on a guess.
- **Versions and restore** from the app, a recycle bin, and two integrity checks per job: a quick
  one (every file is still in the vault, with the right size) and a full one (every file is
  downloaded, decrypted and compared with what was synced).
- **Automatic jobs**: scheduler, file watcher, time windows, `.cuignore`, selective sync, bandwidth
  limit, pre/post-sync hooks. They run inside the app, which stays in the notification area when
  you close the window (the vault is locked, a menu entry reopens the window; an option turns this
  off), and can start without a window when you sign in. A job starts by itself only if you turned
  on its auto-unlock.
- **Ransomware-style anomaly detection**: a sync that would propagate mass changes stops and
  asks.
- **Storage usage dashboard** and system notifications.

### Desktop app and virtual drive

- **Desktop app** with guided first run, browse, upload and download (including drag and drop
  and multiple selection), search by name, jobs, remotes, settings, light and dark themes.
- **Virtual drive** on Linux (FUSE) and Windows (WinFsp): open and edit your files directly,
  also from office suites. Saves go to an encrypted local area and upload in the background; a
  deleted or replaced file goes to the vault's recycle bin. Read-only mode available.
- **Language of the app**: English, Arabic, French, German, Italian, Japanese, Portuguese,
  Russian, Spanish and Chinese, picked from the first screen or from Settings; the app starts in
  the language of the system when it has it. Everything the app shows is in the chosen language,
  even a rare error. English is the reference; the other nine are drafts awaiting review by a
  native speaker. Dates and durations follow the language, and the layout turns right-to-left
  for Arabic.
- **Help translate**, next to the language menu: correct a text with the English reference
  beside it, try your edits live, then save a file or write an email. The app itself sends
  nothing.
- **Report a problem**, one click away in the side bar and on the first screens: write what went
  wrong or an idea, see the whole message (with the version, the system and the language, which
  you can remove), then send it as an email, as a GitHub issue or copy it. The app sends
  nothing by itself, and file names, vault names and paths are never added.
- **Automatic lock** after inactivity, on suspend, on screen lock and when a remote-desktop
  session starts.
- **Signed automatic updates**, installers for Windows (`.exe`, per user), Linux (`.deb`,
  `.AppImage`). The Windows installer speaks the same ten languages (it follows the system
  language) and, when WinFsp is missing, offers to open its download page; the `.deb` suggests
  `fuse3` for the virtual drive.

### Sharing and extra protection

- **Multi-user vaults without a server** (X25519). Removing a member rotates the keys.
- **Share a single file** as an encrypted package, and a **one-way drop box** to receive files
  from people who do not hold the vault keys.
- **Hardware key (FIDO2)** as a quick unlock or as a true second factor; **system keychain**
  unlock.
- **Recovery phrase split into shares** (SLIP-39, K of N) and a **printable recovery kit**.
- **Advanced security section**, all off by default: read-only drive, programs allowed to read
  the drive (Linux), decoy files with an alarm on mass reads (Linux), panic button, failed-unlock
  notice, window hidden from screen capture (Windows), encrypted local state.
  Each option is explained in the app, with what it protects and what it costs.

### Known limits

- The beta is **not code-signed**: Windows SmartScreen asks for confirmation on first launch.
- A message written by the operating system or by a storage server (for example a refused
  network connection) is shown as that system wrote it, in its own language.
- Vaults created with this version are meant to keep opening in future versions.

Report problems through
[GitHub Issues](https://github.com/cryptoupload/CryptoUpload/issues); report vulnerabilities
privately as described in [SECURITY.md](SECURITY.md).
