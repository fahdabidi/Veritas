# GBN-PROTO-013 Phase 4 Local k8s Public Internet Exposure Report

## Run Metadata

- Date: `2026-05-11`
- Workspace: `prototype/gbn-bridge-proto`
- Base commit before this phase: `84e5f4b test: expand pass4 android emulator automation`
- Host shell: Windows PowerShell invoking WSL2 Ubuntu
- Result: `PASS` for Phase 4 tooling, endpoint-contract validation, fixture artifact
  generation, artifact verification, and teardown invalidation
- Live public DNS/router exposure: not executed in this report because no operator-owned
  public run profile was provided

This report does not claim Smoke 2 success. Smoke 2 remains the physical Android phone
run over a real mobile carrier path after a live public run profile is configured.

## Scope

This report covers Pass 4 Phase 4: Local k8s Public Internet Exposure.

The implementation adds:

1. WSL2 operator scripts for public ingress prepare, verify, teardown, and self-test.
2. A shared JSON validator/generator for endpoint descriptors and HostCreator seed
   artifacts.
3. An example `local_k8s_public` run profile.
4. Public endpoint validation that rejects localhost, private IPs, Kubernetes service DNS
   names, expired descriptors, missing TLS binding, and admin/private ports.
5. HostCreator bootstrap seed generation that includes HostCreator public key and
   reachability metadata, while rejecting Publisher/ExitBridge shortcut fields.
6. Publisher public-DHT descriptor snapshot generation for the live Publisher signing
   step.
7. Admin-denial evidence generation and teardown invalidation artifacts.

## Implementation Evidence

| Area | Evidence |
|---|---|
| Prepare script | `prototype/gbn-bridge-proto/infra/scripts/k8s-pass4-public-ingress-prepare.sh` |
| Verify script | `prototype/gbn-bridge-proto/infra/scripts/k8s-pass4-public-ingress-verify.sh` |
| Teardown script | `prototype/gbn-bridge-proto/infra/scripts/k8s-pass4-public-ingress-down.sh` |
| Self-test | `prototype/gbn-bridge-proto/infra/scripts/k8s-pass4-public-ingress-self-test.sh` |
| Shared helper | `prototype/gbn-bridge-proto/infra/scripts/k8s_pass4_public_ingress.py` |
| Example profile | `prototype/gbn-bridge-proto/infra/pass4/public-ingress/run-profile.local-k8s-public.example.json` |

## Validation Command Ledger

| Command | Status | Evidence |
|---|---:|---|
| `bash -n infra/scripts/k8s-pass4-public-ingress-*.sh` | pass | Shell syntax clean |
| `python3 -m py_compile infra/scripts/k8s_pass4_public_ingress.py` | pass | Python helper compiles |
| `infra/scripts/k8s-pass4-public-ingress-self-test.sh` | pass | Fixture prepare, verify, teardown, private-host rejection, and admin-port rejection passed |
| `k8s-pass4-public-ingress-prepare.sh --skip-k8s-check --skip-network-checks` | pass | Fixture artifacts generated under `target/pass4-public-ingress/phase4-fixture-20260511` |
| `k8s-pass4-public-ingress-verify.sh --skip-network-checks` | pass | Required artifacts and endpoint contracts verified |
| `k8s-pass4-public-ingress-down.sh` | pass | Endpoint map invalidated and teardown transcript generated |
| `shellcheck infra/scripts/k8s-pass4-public-ingress-*.sh` | not run | `shellcheck` is not installed in the WSL2 image |
| `git diff --stat -- prototype/gbn-proto/` | pass | No V1 source changes |
| `git diff --stat -- docs/prototyping/Lattice/` | pass | No V1/Lattice doc changes |

## Fixture Artifact Evidence

Fixture artifact root:

```text
prototype/gbn-bridge-proto/target/pass4-public-ingress/phase4-fixture-20260511
```

| Artifact | Size | SHA-256 |
|---|---:|---|
| `public_endpoint_map.json` | 3170 | `833fa0ad009335b088e6110f22c8458e3b86e55e1d84fdc9f5f45115d140a66a` |
| `publisher_public_dht_snapshot.json` | 2229 | `d92f21d707e6618d3c0804239e168fdb98edd0b23a888154872037da827d57ae` |
| `hostcreator_bootstrap_qr.png` | 3993 | `d76c2aacb39477ba5151678acdbca3ff3f283438d17b594b83fe69466ec5ef69` |
| `hostcreator_bootstrap_seed.redacted.json` | 2629 | `ed618ab34be416de2fc1a91b8fcf8684f86aaaf151f7fdbb09a39e9c80fcc9c6` |
| `public_ingress_evidence.json` | 3462 | `435fd945ff684764feb8a56370b8e762c6e358a515d5c85bd49708837fb15eb8` |
| `public_ingress_verify.json` | 392 | `d39a4a47757e34351c6a3c62772feda723a67b7a3ba7bd335b8c9fde38f98eff` |
| `public_ingress_teardown.json` | 360 | `acf66b1a61eda5b82e8702240344a9e4409e055e629eda2b87f54e927d65c82f` |

Additional expected artifacts were generated:

- `hostcreator_bootstrap_qr.svg`
- `hostcreator_bootstrap_qr_payload.txt`
- `hostcreator_bootstrap_seed.json`
- `public_reachability_transcript.txt`
- `admin_denial_transcript.txt`
- `teardown_transcript.txt`
- `public_endpoint_map.invalidated.json`
- `k8s_readiness.json`

## Acceptance Coverage

| Requirement | Status | Evidence |
|---|---:|---|
| Public endpoint map for Publisher, HostCreator, and ExitBridge protocol surfaces | pass | `public_endpoint_map.json` |
| Reject non-public hosts and cluster-local names | pass | Self-test private-host negative case |
| Reject public admin/private ports | pass | Self-test admin-port negative case |
| HostCreator QR seed contains HostCreator public key and reachability only | pass | `hostcreator_bootstrap_seed.redacted.json`; seed validator rejects Publisher/bridge shortcut fields |
| Publisher public-DHT descriptor snapshot exists | pass | `publisher_public_dht_snapshot.json` |
| Admin-denial transcript exists | pass | `admin_denial_transcript.txt` |
| Teardown invalidates endpoint descriptors | pass | `public_endpoint_map.invalidated.json`, `teardown_transcript.txt` |
| V1 preserved | pass | No diff under `prototype/gbn-proto/` or `docs/prototyping/Lattice/` |

## Live Run Notes

The fixture run used `--skip-network-checks` and `--skip-k8s-check` because no live
public DNS/router/port-forward profile was available in this session. For Phase 5, replace
the example profile with real public hosts, certificate fingerprints, descriptor expiry,
and admin-denial URLs, then rerun prepare and verify without the skip flags.

`qrencode` is not installed in the current WSL2 image. The helper therefore generated a
deterministic placeholder PNG plus the exact QR payload text. Installing `qrencode` before
the live Phase 5 run will make the script emit a scannable QR PNG directly.
