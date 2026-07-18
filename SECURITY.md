# Security policy

Jadren accepts responsible security reports for the compiler, runtime, native
ABI, Unity packages, editor tooling, GPU backends, and release artifacts.

## Reporting a vulnerability

Do not publish an exploit, sensitive reproducer, credentials, or private user
data in a public issue. Use the repository host's private vulnerability
reporting or security-advisory channel when it is enabled.

A useful report includes:

- affected version or commit;
- platform and toolchain;
- minimal reproducer;
- expected and actual behaviour;
- potential impact;
- whether the issue is already public.

Do not send secrets or unnecessary personal data. Prefer an artifact hash over a
sensitive binary when possible.

## Disclosure process

The maintainers will confirm receipt, assess affected versions, prepare a fix
and regression test, and coordinate disclosure after a mitigation is available.
Security fixes that change language semantics must also include a documented
migration path.

Before the first public release, private vulnerability reporting or a verified
security contact must be available in the release metadata.
