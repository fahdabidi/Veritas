# Funding Veritas

Veritas is an open-source project building censorship-resilient media infrastructure for creators, publishers, and witnesses operating under censorship, network disruption, and adversarial pressure.

Current support path:

- GitHub Sponsors: [github.com/sponsors/fahdabidi](https://github.com/sponsors/fahdabidi)

## GitHub Sponsors Profile Copy

### Short Bio

```text
Building Veritas, open-source censorship-resilient media infrastructure for secure creation, publication, and distribution.
```

### Introduction

```text
Veritas is building open-source infrastructure for media creation and distribution under censorship, network disruption, and hostile state pressure.

The work spans two architecture tracks:
- Lattice: the V1 onion-mode baseline
- Conduit: the V2 bridge-mode architecture now in active prototyping

Sponsorship helps fund the practical work required to make the project real:
- protocol design and implementation
- security review and audit preparation
- prototype infrastructure and validation
- documentation, release engineering, and maintenance

Support sustains the work, but it does not buy editorial control, protocol governance, or architectural influence.
```

## Suggested GitHub Sponsors Tiers

### $5/month — Support Veritas

Help sustain ongoing documentation, architecture, testing, and maintenance work across the project.

### $25/month — Back the Roadmap

Support milestone planning, prototyping, release preparation, and steady progress across both Lattice and Conduit.

### $100/month — Infrastructure Sponsor

Help cover cloud validation, test infrastructure, observability, and release-engineering costs. Sponsors at this level may be listed in the public acknowledgments section if they want to be named.

### $500/month — Project Sponsor

Provide meaningful support for sustained implementation, security review preparation, and prototype operations. Sponsors at this level may be listed in the public acknowledgments section if they want to be named.

## What Funding Supports

Funding helps sustain work across both active architecture tracks:

- `Lattice`: V1 onion-mode baseline maintenance and validation
- `Conduit`: V2 bridge-mode design, prototyping, and testing

Funding is intended to support:

- protocol design and implementation
- security review and audit preparation
- prototype infrastructure and cloud validation costs
- testing, observability, and release engineering
- documentation, research, and threat-model analysis

## Independence

Financial support does not buy:

- editorial control
- protocol governance control
- special treatment in security or architectural decisions
- ownership of the project roadmap

Support sustains the work. It does not override the project's security, architectural, or governance principles.

## Scope

Funding links are for direct support of the open-source project itself. They are not for political fundraising, charity drives, or unrelated campaigns.

## Future Funding Channels

Additional transparent, project-directed funding channels may be added later as the project matures.

## Sponsor Acknowledgments

Veritas may publicly acknowledge sponsors who opt in to being named.

Suggested acknowledgment groups:

- Supporters
- Infrastructure Sponsors
- Project Sponsors

Acknowledgment is a thank-you, not an endorsement relationship and not a governance mechanism.

README acknowledgments can be automated.

- Workflow: `.github/workflows/sync-sponsors.yml`
- Updater script: `tools/update_sponsors_readme.py`
- Data source: GitHub Sponsors GraphQL API
- Output target: the sponsor acknowledgment block in `README.md`

The automation is designed to publish only public sponsors who opted in to be named.

To enable the automation, add this repository Actions secret:

- `SPONSORS_READ_TOKEN`

Use a token from the sponsored account that can call GitHub's GraphQL API for Sponsors data. Once the secret is set, you can run the `Sync Sponsors` workflow manually or let the scheduled run keep the README section current.

Suggested README-style acknowledgment block:

```md
## Sponsor Acknowledgments

Veritas is sustained by sponsors who help fund architecture work, testing, security preparation, and prototype infrastructure.

With sponsor permission, public acknowledgments will appear here.

### Supporters
- Coming soon

### Infrastructure Sponsors
- Coming soon

### Project Sponsors
- Coming soon
```
