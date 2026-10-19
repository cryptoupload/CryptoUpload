# Security policy

## CryptoUpload has not yet had an independent security audit

Do not rely on it as the only protection for information whose disclosure would seriously harm
you. The goal is an independent external audit, based
on a working beta and the open
[Unified Vault Format](https://github.com/encryption-alliance/unified-vault-format).

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
