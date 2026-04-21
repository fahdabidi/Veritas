# Contributing to Veritas

Thanks for contributing. Veritas is developing two related tracks:

- `Lattice`: the V1 onion-mode baseline
- `Conduit`: the V2 bridge-mode architecture now under active prototyping

This top-level guide is the repo entry point for contributors. Prototype-specific details still live in [prototype/gbn-proto/CONTRIBUTING.md](prototype/gbn-proto/CONTRIBUTING.md).

## Where Work Lives

- protocol, runtime, and infra prototype code: [prototype/gbn-proto](prototype/gbn-proto/)
- architecture, security, and prototype plans: [docs](docs/)
- release automation and repo metadata: [.github](.github/)

## Before Opening a PR

For substantial changes, open an issue first if you are changing:

- protocol or wire-format behavior
- cryptographic design
- trust or anonymity assumptions
- deployment or release automation
- architecture or security documents

Small fixes, typo corrections, and focused test improvements can usually go straight to PR.

## Local Development

Most code work happens from `prototype/gbn-proto/`.

```bash
cd prototype/gbn-proto
cargo test --workspace
cargo fmt --check
cargo clippy --workspace -- -D warnings
```

If you change docs, check links and keep naming consistent across:

- `README.md`
- `docs/architecture/`
- `docs/security/`
- `docs/prototyping/`

## Pull Request Expectations

Every PR should:

- describe what changed and why
- link the relevant issue, document, or prototype phase when applicable
- include tests when behavior changes
- avoid unrelated cleanup in the same PR
- preserve the frozen V1 baseline unless the change is explicitly approved as V1 maintenance

## AI-Assisted Contributions

AI-assisted contributions are allowed, but the contributor remains responsible for every line submitted.

If AI assistance was substantial, disclose it in the PR description and confirm that you:

- reviewed the generated output
- tested it appropriately
- understand the changes being proposed

## Security-Sensitive Changes

For cryptography, routing, trust, or censorship-resistance logic:

- do not introduce homegrown cryptographic primitives
- explain the threat-model impact in the PR description
- prefer design discussion before implementation

## License

This repository is currently distributed under the Apache 2.0 license. By contributing, you agree that your contributions are submitted under the repository license unless explicitly stated otherwise.

For deeper prototype-specific policy language, see [prototype/gbn-proto/CONTRIBUTING.md](prototype/gbn-proto/CONTRIBUTING.md).
