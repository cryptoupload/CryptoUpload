# Connecting a storage provider

CryptoUpload encrypts on your device and stores only ciphertext on the storage you choose.
This page shows how to connect each supported provider, from the desktop app
(**Remotes → Add remote**).

Credentials are never stored in clear text: they live in an encrypted file,
optionally unlocked through the system keychain.

| Provider | What you need | Status |
|---|---|---|
| Local folder / external disk | A path | Stable |
| S3-compatible (AWS S3, MinIO, Backblaze B2, Wasabi…) | Endpoint, bucket, access key, secret key | Stable on AWS and MinIO. On Backblaze B2 and Wasabi, CryptoUpload checks at connection time that the provider enforces exclusive writes and refuses the remote if it does not |
| SFTP (NAS, VPS, any SSH server) | Host, user, password or private key | Stable |
| WebDAV (Nextcloud, ownCloud…) | URL, user, password | Stable |
| Google Drive | Your own OAuth client (free) | Stable |
| Dropbox | Your own app key (free) | Experimental |
| OneDrive | Your own app registration (free) | Experimental |

Why "your own" OAuth client? CryptoUpload has no server and no company account sitting between
you and your provider. You register a free application in your own account, so nobody else can
ever revoke, throttle or observe your access.

After any of the sections below, you can verify the connection end to end: create a vault on the
remote, add a file, lock the vault, open it again and check that the file opens.

## S3-compatible storage

In **Remotes → Add remote**, choose a preset (custom, Backblaze B2, Wasabi), then fill in endpoint,
region, bucket, optional prefix, path-style addressing (on for MinIO, off for AWS/B2/Wasabi),
access key and secret key. A plain-HTTP endpoint
on a public address is refused; plain HTTP is accepted only for local and private-network
addresses.

**Immutable backups (Object Lock).** Turn on *Immutable backups (Object Lock)* to make every uploaded
object undeletable for the retention period you choose. The bucket must have been created with
Object Lock enabled (`mc mb --with-lock …`, or `--object-lock-enabled-for-bucket` on AWS);
otherwise the remote is refused with an explicit message and nothing is saved.

**Trying it locally with MinIO** (no account, nothing leaves your computer):

```bash
mkdir -p ~/minio-test/data
curl -sSL https://dl.min.io/server/minio/release/linux-amd64/minio -o ~/minio-test/minio
curl -sSL https://dl.min.io/client/mc/release/linux-amd64/mc -o ~/minio-test/mc
chmod +x ~/minio-test/minio ~/minio-test/mc
MINIO_ROOT_USER=minioadmin MINIO_ROOT_PASSWORD=minioadmin123 \
  ~/minio-test/minio server ~/minio-test/data --address ":9000" &
~/minio-test/mc alias set localtest http://127.0.0.1:9000 minioadmin minioadmin123
~/minio-test/mc mb localtest/cryptoupload-test
```

Endpoint `http://127.0.0.1:9000`, region `us-east-1`, bucket `cryptoupload-test`, path-style on.

## SFTP

In **Remotes → Add remote**, choose SFTP, then fill in host, port, user and a password or the
path of a private key. The first connection shows the server's host key fingerprint and asks
whether you trust it. From then on a different host key is always refused — there is no option to
bypass it.

Exclusive writes rely on the SFTPv3 rule that `RENAME` fails when the destination exists. Real
POSIX servers (OpenSSH) honour it; object-storage gateways that emulate SFTP may not.

## WebDAV

In **Remotes → Add remote**, choose WebDAV, then fill in base URL, user and password. HTTPS is the
default; plain HTTP requires an explicit opt-in and shows a warning, because Basic credentials
would travel unencrypted.

A throw-away local server for a quick try:

```bash
rclone serve webdav --addr 127.0.0.1:8098 --user testuser --pass testpass /tmp/webdav-test
```

## Google Drive

Register a free "Desktop app" OAuth client once:

1. Open [console.cloud.google.com](https://console.cloud.google.com) and create a project.
2. **APIs & Services → Library** → enable **Google Drive API**.
3. **APIs & Services → OAuth consent screen** → type *External*; add your own Google address as
   a test user.
4. **APIs & Services → Credentials → Create credentials → OAuth client ID** → application type
   **Desktop app**. This type is required: it is the only one that allows a redirect to any port
   on `127.0.0.1`.
5. Copy the **Client ID** and **Client secret**.

In **Remotes → Add remote**, choose Google Drive, paste the two values and follow the browser
window that opens to grant consent (an "unverified app" notice is normal for an app in test
mode). Revoking access at [myaccount.google.com/permissions](https://myaccount.google.com/permissions)
and adding the remote again restores a clean connection.

## Dropbox

Register a free app once:

1. Open [dropbox.com/developers/apps](https://www.dropbox.com/developers/apps) → **Create app**.
2. API: **Scoped access**. Folder access: **App folder** — the only type supported: the app sees
   `/Apps/<AppName>` and nothing else in your Dropbox.
3. In **Permissions**, tick exactly `files.metadata.read`, `files.metadata.write`,
   `files.content.read`, `files.content.write`, then **Submit**.
4. In **Settings → OAuth 2 → Redirect URIs**, add exactly `http://127.0.0.1:53682/`. The port is
   fixed because Dropbox requires a byte-for-byte match.
5. Copy the **App key**: it is the Client ID. No client secret is needed.

In **Remotes → Add remote**, choose Dropbox and paste the App key. If port 53682 is busy, close the program using it
(often `rclone`) and try again.

## OneDrive

Register a free application once (a personal Microsoft account is enough):

1. Open [entra.microsoft.com](https://entra.microsoft.com) → **App registrations → New
   registration**.
2. Supported account types: **Accounts in any organizational directory and personal Microsoft
   accounts**. Do not set a redirect URI on this screen.
3. **Authentication → Add a platform → Mobile and desktop applications** (not "Web": that type
   demands a client secret).
4. Copy the **Application (client) ID** from **Overview**.

In **Remotes → Add remote**, choose OneDrive, paste the Client ID, leave the client secret empty
and keep `common` as the tenant. Then choose where the vault lives:

- **Dedicated app folder** (recommended): CryptoUpload sees only its own private folder.
- **Whole OneDrive**: needed only for OneDrive for Business accounts, where the dedicated folder
  does not exist.
