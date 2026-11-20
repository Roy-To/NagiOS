# Security Policy

Nagi OS is a Developer Preview and experimental operating system. It is not
currently a production security boundary and does not provide production
security guarantees.

## Reporting a vulnerability

Do not disclose credentials, private keys, access tokens, personal data, or
other sensitive material in a public issue, pull request, commit, or log.

Report security vulnerabilities privately through GitHub's
**Private vulnerability reporting**: open the repository's **Security** tab,
go to **Advisories**, and choose **Report a vulnerability**. Please do not open
a public issue for a suspected vulnerability.

Include the affected revision, component, reproduction conditions, and impact
assessment in the private report. Do not include real credentials or personal
data; use placeholders instead.

If private reporting is unavailable, contact the repository owner through a
private GitHub channel and provide only the minimum information needed to
establish a private conversation.

For community conduct concerns, see [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
Accessibility barriers are covered by [ACCESSIBILITY.md](ACCESSIBILITY.md).

## Supported versions

Until Nagi OS 1.0, only the latest version is supported. Older versions do not
receive fixes. A support period for each version will be defined at 1.0.

## Response targets

These are targets, not guarantees
([DEC-118](docs/product-decisions/DEC-118-support-period-and-vulnerability-response.md)):

| Kind of vulnerability | Target for shipping a fix |
| --- | --- |
| Actively exploited, or details already public | Within 7 days |
| Serious, not yet public (privately reported) | Within 30 days |
| Other | Next regular update (at the latest within 90 days) |

The clock starts when the project becomes aware of the vulnerability.
