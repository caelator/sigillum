# Support

Sigillum does not currently have a supported stable release or a commercial
support contract.

## Questions and bugs

Use [GitHub Issues](https://github.com/caelator/sigillum/issues) for reproducible
bugs, documentation problems, and scoped feature proposals. Search existing
issues first and include the commit tested, operating system, redacted logs, and
the smallest safe reproduction.

Never attach seed phrases, private keys, API tokens, session tokens, private
wallet addresses, unredacted data directories, or customer information.

## Security vulnerabilities

Do not open a public issue. Follow [SECURITY.md](SECURITY.md) and use GitHub's
private vulnerability reporting when available.

## Operational incidents

Sigillum is source-evaluation software today. If value may be at risk, stop the
daemon and gateway, preserve the data directory and logs without publishing
them, avoid retrying an ambiguous transaction, and verify chain state through
an independent source before taking further action.
