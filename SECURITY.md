# Security policy

## CryptoUpload has not yet had an independent security audit

Do not rely on it as the only protection for information whose disclosure would seriously harm
you. The goal is an independent external audit, based
on a working beta and the open
[Unified Vault Format](https://github.com/encryption-alliance/unified-vault-format).

## Check a download

Official installers come only from the
[Releases page](https://github.com/cryptoupload/CryptoUpload/releases) of this repository. They
are built by the project's CI from the published code, and each one has a `.sig` file next to it:
a signature made with the project's update key. The app's updater checks every update against
the same public key and refuses anything not signed with it. Anyone can build the code, but a
build from a fork or from another site is not signed by the project.

The public key is:

```
RWTt1I9t8dl/wTIsTZ/5C67P1M+ovIJbJfS1drkJRVnbYXplgUPLRQvb
```

To check a file by hand with [minisign](https://jedisct1.github.io/minisign/), download it with
its `.sig` (the one with the version in its name; the examples use 0.1.0-beta.2), then on Linux:

```bash
base64 -d CryptoUpload_0.1.0-beta.2_amd64.deb.sig > file.minisig
minisign -V -P RWTt1I9t8dl/wTIsTZ/5C67P1M+ovIJbJfS1drkJRVnbYXplgUPLRQvb \
  -m CryptoUpload_0.1.0-beta.2_amd64.deb -x file.minisig
```

On Windows, in PowerShell:

```powershell
[IO.File]::WriteAllBytes("$PWD\file.minisig", [Convert]::FromBase64String((Get-Content CryptoUpload_0.1.0-beta.2_x64-setup.exe.sig)))
minisign -V -P RWTt1I9t8dl/wTIsTZ/5C67P1M+ovIJbJfS1drkJRVnbYXplgUPLRQvb -m CryptoUpload_0.1.0-beta.2_x64-setup.exe -x file.minisig
```

`Signature and comment signature verified` means the file is exactly the one the project
published. Any other result: do not install it, and report it to security@cryptoupload.org.

## Reporting a vulnerability

**Do not open a public issue for a vulnerability.** Use one of these private channels:

1. [GitHub private vulnerability reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing/privately-reporting-a-security-vulnerability)
   on the repository (**Security → Report a vulnerability**).
2. Email **security@cryptoupload.org**. This address is for vulnerabilities only.

If you can, include the affected version, steps to reproduce, the estimated impact
(confidentiality, integrity, availability) and whether you have a proof of concept.

## In scope

- `cu-crypto`, `cu-format`: any way to obtain plaintext, forge a tag, bypass the end-of-file
  block rule, or recover a key without the password or the recovery phrase.
- `cu-sync`: any scenario of silent data loss.
- `cu-secrets` and credential handling: any way to exfiltrate a saved credential.
- Any storage backend (`cu-backend-*`): unauthenticated requests, path traversal, SSRF.
- The desktop app.

## Out of scope for now

- Findings that require continuous physical access to an already unlocked device.
- Rate limiting or denial of service against third-party storage providers.
- General robustness bugs that do not put real data at risk: open a normal issue.

## Timeline

There is no bug bounty. Receipt is confirmed within 5 working days, and a disclosure window is
agreed with the reporter — by default **90 days** from confirmation.

## Supported versions

| Version | Supported |
|---|---|
| `0.1.0-beta.*` | Yes — the only active series |
| `main` | Yes — security fixes land here first |
