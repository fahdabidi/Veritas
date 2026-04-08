# Contributing to the Global Broadcast Network (GBN)

Thank you for your interest in contributing to the GBN. This project aims to build censorship-resistant infrastructure, and every contribution — whether code, documentation, security review, or testing — matters.

---

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [How to Contribute](#how-to-contribute)
- [AI-Assisted Contributions](#ai-assisted-contributions)
- [Contributor License Agreement](#contributor-license-agreement)
- [Development Workflow](#development-workflow)
- [Licensing](#licensing)

---

## Code of Conduct

Contributors are expected to act professionally and respectfully. Harassment, discrimination, and bad-faith behavior will not be tolerated. Detailed Code of Conduct to be published before first public release.

---

## How to Contribute

### Reporting Issues

Open a GitHub Issue with:
- A clear title describing the problem
- Steps to reproduce (if applicable)
- Which prototype phase or component is affected
- Your environment (OS, Rust version)

### Submitting Code Changes

1. **Open an issue first** — describe what you want to change and why
2. **Fork the repository** and create a feature branch: `git checkout -b feature/my-change`
3. **Write tests** — all changes must include or update tests (`cargo test --workspace`)
4. **Follow the code style** — run `cargo fmt` and `cargo clippy` before submitting
5. **Submit a Pull Request** with a clear description linking the issue

### Reviewing Security Documents

Security reviews are especially valuable. If you find a flaw in any `GBN-SEC-*` document, open an issue labeled `security` with your analysis.

---

## AI-Assisted Contributions

### Policy

This project **welcomes contributions created with the assistance of AI coding tools** (GitHub Copilot, Claude, Gemini, ChatGPT, custom agents, etc.). There is no prohibition on using AI to write code, documentation, or tests.

However, **AI assistance does not change contributor responsibilities.** You are accountable for every line you submit, regardless of how it was generated.

### Requirements for AI-Assisted Contributions

When submitting code that was substantially generated or directed by an AI tool, the contributor MUST:

1. **Review and understand** all generated code before submitting. You are the author; the AI is the tool.

2. **Test the output** — AI-generated code must pass the same `cargo test --workspace` bar as human-written code. AI does not get a quality pass.

3. **Disclose AI involvement** in the Pull Request description when the AI's contribution was substantial (not just autocomplete). Example:

   ```
   ## AI Disclosure
   This PR was developed with the assistance of [Claude/Copilot/Gemini].
   The contributor reviewed, tested, and takes responsibility for all code.
   ```

4. **Assert authorship** — By submitting the PR, you certify that you exercised meaningful creative judgment in directing, reviewing, and editing the AI's output, and you claim authorship under the Contributor License Agreement below.

### Why This Matters

Open source licenses are copyright licenses. Copyright requires human authorship. By certifying that *you* — the human — directed the AI and exercised editorial judgment, you establish the legal foundation that makes the project's license enforceable. Without this, AI-generated code could be considered uncopyrightable, which would make the license a legal nullity.

---

## Contributor License Agreement (CLA)

By submitting a Pull Request to this repository, you agree to the following terms:

### 1. Definitions

- **"Contribution"** means any source code, documentation, configuration, test, or other work you submit to this project via Pull Request, patch, or any other mechanism.
- **"You"** means the individual submitting the Contribution.
- **"AI Tool"** means any artificial intelligence or machine learning system used to generate, suggest, or assist in creating any part of a Contribution, including but not limited to code assistants, chat-based coding agents, and automated code generators.

### 2. Grant of Copyright License

You hereby grant to the project maintainers and all recipients of the project a perpetual, worldwide, non-exclusive, no-charge, royalty-free, irrevocable copyright license to reproduce, prepare derivative works of, publicly display, publicly perform, sublicense, and distribute your Contributions and any derivative works.

### 3. Grant of Patent License

You hereby grant to the project maintainers and all recipients of the project a perpetual, worldwide, non-exclusive, no-charge, royalty-free, irrevocable patent license to make, have made, use, offer to sell, sell, import, and otherwise transfer the Contribution, where such license applies only to patent claims licensable by you that are necessarily infringed by your Contribution alone or in combination with the project.

### 4. Representation of Authorship

You represent that:

(a) You are legally entitled to grant the above licenses.

(b) Each Contribution is **your original work of authorship.** If any portion was generated with the assistance of an AI Tool, you represent that you exercised **meaningful creative judgment** in one or more of the following ways:
   - Providing detailed architectural specifications, requirements, or instructions that substantially directed the AI Tool's output
   - Selecting, arranging, and editing the AI Tool's output to achieve a specific technical result
   - Reviewing the AI Tool's output for correctness, security, and fitness for purpose
   - Making material modifications to the AI Tool's raw output

(c) You understand that the project relies on copyright ownership to enforce its open source license, and that your assertion of authorship is necessary for this enforcement.

### 5. Disclosure of AI Tool Usage

If an AI Tool was used in the creation of your Contribution beyond trivial autocomplete suggestions, you agree to disclose this in your Pull Request description, identifying the tool used and the general nature of its contribution.

### 6. No Obligation to Accept

The project maintainers are under no obligation to accept any Contribution. Submission of a Contribution does not create any right to have it included.

### 7. Future License Changes

This CLA is compatible with a future transition from Apache-2.0 to AGPL-3.0 for production releases. By signing this CLA, you acknowledge that the project may relicense under AGPL-3.0-or-later at the maintainers' discretion. Your Contribution will be available under whichever license the project adopts, within the scope of the licenses you have granted above.

---

## Development Workflow

### Branch Naming

- `feature/<description>` — new functionality
- `fix/<description>` — bug fixes
- `security/<description>` — security-related changes
- `docs/<description>` — documentation only

### Commit Messages

Use conventional commit format:

```
feat(mcn-crypto): implement X25519 ECDH key exchange

- Generate Publisher X25519 key from Ed25519 seed
- Create ephemeral keypair per upload session
- Derive session key via HKDF-SHA256

AI-assisted: yes (Claude, directed by contributor)
```

### Testing Requirements

All PRs must:
- [ ] Pass `cargo test --workspace`
- [ ] Pass `cargo clippy --workspace -- -D warnings`
- [ ] Pass `cargo fmt --check`
- [ ] Include new tests for new functionality
- [ ] Not reduce existing test coverage

### Security-Sensitive Changes

Changes to cryptographic code (`mcn-crypto`, `gbn-protocol/src/crypto.rs`) or protocol definitions require:
- Review by at least 2 maintainers
- Explicit security analysis in the PR description
- No AI-generated cryptographic primitives (use audited crates only)

---

## Licensing

### Current: Apache-2.0 (Prototype Phase)

This prototype is licensed under the **Apache License, Version 2.0**. This provides:
- A permissive base for rapid prototyping and experimentation
- An explicit patent grant protecting all contributors and users
- Broad compatibility with downstream projects and dependencies

### Future: AGPL-3.0 (Production)

When the project transitions from prototype to production, the license will transition to **AGPL-3.0-or-later**. This provides:
- **Network copyleft** — anyone operating a modified GBN relay, storage node, or service must share their changes
- Protection against hostile closed-source forks
- Alignment with the project's sovereignty and transparency principles

The CLA above includes a forward-looking grant that permits this transition without requiring re-consent from past contributors.

---

## Questions?

Open an issue labeled `question` or reach out to the maintainers. We are happy to help new contributors find the right place to start.
