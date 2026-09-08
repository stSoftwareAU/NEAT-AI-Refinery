# Security Policy

Please report suspected vulnerabilities privately through GitHub's security
reporting facilities where available.

NEAT-AI-Refinery processes local training-data files. It must not require
network access for normal corpus transformation, and it must never modify the
source corpus in place.

Do not commit credentials, tokens, private datasets, or proprietary downstream
configuration to this repository.

## Actively-exploited advisories

Dependency updates normally arrive through the weekly `cargo-upgrade.yml` PR.
When an advisory against a crate already in `Cargo.lock` is being exploited and
cannot wait for that cycle, follow the expedited path in
[`docs/incident-response.md`](docs/incident-response.md): who authorises the
bypass, which gates still must pass, and how the fix PR is flagged for
expedited review.
