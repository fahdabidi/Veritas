# GBN-PROTO-013 - Execution Phase 4 - Local k8s Public Internet Exposure

**Status:** Completed
**Last Updated:** 2026-05-11
**Parent Plan:** [GBN-PROTO-013](GBN-PROTO-013-Conduit-Mobile-Creator-Public-Internet-Validation-Execution-Plan.md)
**Depends On:** Phases 1-3 complete

## Objective

Expose the existing local k8s Publisher, HostCreator bootstrap surface, and ExitBridge
protocol endpoints to the public internet so a physical Android phone on a mobile network
can run the creator flow against the local cluster.

This phase does not run the mobile validation itself. It prepares a controlled, temporary,
auditable public protocol path while keeping every admin surface private.

At completion:

- local k8s Publisher protocol endpoints are reachable from outside the LAN;
- HostCreator bootstrap endpoint metadata can be encoded in `BootstrapDHTQRCode`;
- local k8s ExitBridge public endpoints are represented in Publisher-signed DHT entries;
- public endpoint descriptors contain only mobile-reachable addresses;
- admin listeners remain inaccessible from public internet;
- teardown returns the cluster to private-only exposure;
- V1 remains untouched.

Update the parent plan status tracker when this phase is complete.

Phase 4 completion means the operator tooling, endpoint contract validation, QR/seed
artifact generation, admin-denial checks, teardown invalidation, and fixture automation
exist and pass. A live outside-LAN public DNS/router run still requires an operator
provided run profile with real public hosts and is the precondition for Phase 5 mobile
validation; Phase 4 does not claim Smoke 2 success by itself.

---

## Exposure Model

Phase 4 exposes protocol traffic only:

| Surface | Public Exposure | Notes |
|---|---|---|
| Publisher authority protocol | Required | Used by HostCreator and ExitBridge registration/progress paths |
| Publisher receiver protocol | Required | Used by upload/dummy delivery result paths |
| HostCreator bootstrap protocol | Required | Encoded in QR seed for mobile NewCreator first contact |
| ExitBridge ingress / punch endpoints | Required | Mobile NewCreator uses signed public bridge descriptors |
| Admin HTTP listeners | Forbidden | Stay reachable only through WSL2/k8s operator paths |
| Observability dashboards | Forbidden by default | Use WSL2 access, not public ingress |

The exposure must preserve normal protocol authentication, signatures, ChainID, and
encryption semantics. It must not introduce a public admin shortcut that allows the phone
or an internet client to call `seed-new-creator`, `local-dht`, `send-dummy`, or reset
state directly.

---

## Public Ingress Options

The preferred validation path is a direct public endpoint to the local cluster:

1. router or firewall port-forward from a public IP/DNS name to the WSL2/k3d ingress;
2. TLS termination with a validation certificate or pinned self-signed test certificate;
3. explicit TCP and UDP port mapping for Publisher, HostCreator, and ExitBridge protocol
   ports;
4. teardown of every public mapping after the run.

If direct router port-forwarding is unavailable, a temporary public relay VM may be used
as a labeled fallback only when it forwards protocol traffic without changing the Conduit
protocol contract. The phone still talks to public internet endpoints, and the report must
label the run as `public_relay_fallback`.

Any fallback that hides UDP reachability, replaces bridge endpoint descriptors with private
VPN addresses, or changes the signed DHT endpoint semantics cannot be used for Pass 4
sign-off.

---

## Endpoint Descriptor Contract

Every public endpoint that can appear in a signed DHT entry must have a descriptor:

```json
{
  "endpoint_id": "pass4-local-exitbridge-01",
  "actor_id": "exitbridge-01",
  "role": "exit_bridge",
  "profile": "local_k8s_public",
  "public_host": "bridge-01.example.test",
  "tcp_port": 443,
  "udp_port": 31001,
  "tls_sni": "bridge-01.example.test",
  "certificate_fingerprint": "sha256:...",
  "reachability_class": "direct",
  "expires_at_ms": 0,
  "chain_id": "pass4-public-ingress-..."
}
```

Rules:

- `public_host` must resolve from outside the LAN.
- `public_host` must not be a Kubernetes service DNS name, pod IP, `localhost`, private
  RFC1918-only address, or WSL-only address.
- TLS/SNI or certificate fingerprint must be present for HTTPS/TLS endpoints.
- UDP endpoints must be explicitly represented when bridge punch/ingress uses UDP.
- Descriptors must be signed into Publisher/creator/bridge DHT entries only after public
  reachability checks pass.
- Descriptors are time-bounded and tied to a validation run id.

---

## HostCreator QR Producer

Phase 4 adds or completes the HostCreator-side `BootstrapDHTQRCode` producer against the
public endpoint map.

Required operator flow:

1. Seed the local k8s HostCreator through private WSL2/k8s tooling.
2. Generate the public endpoint descriptor for HostCreator bootstrap first contact.
3. Run the QR producer from WSL2 or private admin tooling.
4. Producer validates that the HostCreator endpoint is mobile-reachable.
5. Producer writes:
   - QR PNG/SVG;
   - canonical `HostCreatorDhtSeed` payload;
   - redacted evidence manifest;
   - ChainID and payload hash.

The QR payload contains HostCreator public key plus mobile-reachable HostCreator DHT
information only. It must not contain Publisher DHT, Publisher public key, ExitBridge DHT,
admin URLs, private keys, or arbitrary HostCreator local-DHT dumps.

---

## Public Endpoint Readiness Checks

Add a WSL2 operator script:

```text
prototype/gbn-bridge-proto/infra/scripts/k8s-pass4-public-ingress-prepare.sh
```

Expected behavior:

1. Guard for WSL2 Ubuntu.
2. Confirm local k8s Pass 3 topology is running.
3. Confirm Phase 1 strict bootstrap and SendDummy gates have passed or explicitly rerun
   them.
4. Read public ingress configuration from a run-profile JSON file.
5. Verify DNS resolution from a public resolver.
6. Verify TCP/TLS reachability from outside the cluster.
7. Verify UDP reachability where required by the bridge protocol.
8. Generate public endpoint descriptors.
9. Initialize or refresh Publisher-side signed DHT entries with public endpoint metadata.
10. Generate `BootstrapDHTQRCode` for HostCreator.
11. Emit `public_endpoint_map.json` and `public_ingress_evidence.json`.

Add a companion verification script:

```text
prototype/gbn-bridge-proto/infra/scripts/k8s-pass4-public-ingress-verify.sh
```

It must fail if any admin listener is reachable from the public side.

Implemented entrypoints:

```text
prototype/gbn-bridge-proto/infra/scripts/k8s-pass4-public-ingress-prepare.sh
prototype/gbn-bridge-proto/infra/scripts/k8s-pass4-public-ingress-verify.sh
prototype/gbn-bridge-proto/infra/scripts/k8s-pass4-public-ingress-down.sh
prototype/gbn-bridge-proto/infra/scripts/k8s-pass4-public-ingress-self-test.sh
prototype/gbn-bridge-proto/infra/pass4/public-ingress/run-profile.local-k8s-public.example.json
```

The scripts accept an operator run-profile JSON, reject non-public endpoint descriptors,
reject public admin/private ports, generate the HostCreator bootstrap seed, generate QR
artifacts, produce the Publisher public-DHT descriptor snapshot, verify the artifacts,
and invalidate the temporary endpoint map during teardown. If `qrencode` is installed in
WSL2, the QR PNG is generated as a scannable QR. Without `qrencode`, the script emits a
deterministic placeholder PNG plus the exact QR payload text so the live operator can
generate the scannable QR before Phase 5.

---

## Security And Teardown

Public exposure is temporary and validation-scoped:

- allowlist source IPs when possible, but do not rely on allowlisting as the only control
  because the phone's carrier IP may change;
- enforce protocol-level authentication and signed DHT validation;
- use short-lived public endpoint descriptors;
- log every external protocol request with ChainID when available;
- block public access to admin HTTP, shell, metrics mutation endpoints, and Kubernetes API;
- remove router/firewall/tunnel rules after validation;
- archive before/after public exposure evidence.

Teardown script:

```text
prototype/gbn-bridge-proto/infra/scripts/k8s-pass4-public-ingress-down.sh
```

It must remove temporary public forwarding and invalidate the run-profile endpoint
descriptors.

---

## Validation

Run from WSL2 Ubuntu:

```bash
uname -a | grep -i microsoft >/dev/null || { echo "Pass 4 tooling requires WSL2 Ubuntu" >&2; exit 1; }

cd prototype/gbn-bridge-proto
infra/scripts/k8s-up.sh
infra/scripts/k8s-observability-up.sh
infra/scripts/k8s-smoke-bootstrap-strict-v4.sh --require-observability
infra/scripts/k8s-smoke-senddummy-strict-v4.sh --require-observability

infra/scripts/k8s-pass4-public-ingress-prepare.sh \
  --profile local_k8s_public \
  --run-id pass4-local-public-$(date +%Y%m%d-%H%M%S) \
  --config infra/pass4/public-ingress/run-profile.local-k8s-public.example.json

infra/scripts/k8s-pass4-public-ingress-verify.sh \
  --artifact-dir target/pass4-public-ingress/<run-id> \
  --require-no-public-admin \
  --require-hostcreator-qr \
  --require-public-dht-endpoints
```

Fixture validation, used when no real public router/DNS mapping is available yet:

```bash
cd prototype/gbn-bridge-proto
infra/scripts/k8s-pass4-public-ingress-self-test.sh

infra/scripts/k8s-pass4-public-ingress-prepare.sh \
  --config infra/pass4/public-ingress/run-profile.local-k8s-public.example.json \
  --run-id phase4-fixture-20260511 \
  --artifact-dir target/pass4-public-ingress/phase4-fixture-20260511 \
  --skip-k8s-check \
  --skip-network-checks

infra/scripts/k8s-pass4-public-ingress-verify.sh \
  --artifact-dir target/pass4-public-ingress/phase4-fixture-20260511 \
  --require-no-public-admin \
  --require-hostcreator-qr \
  --require-public-dht-endpoints \
  --skip-network-checks

infra/scripts/k8s-pass4-public-ingress-down.sh \
  --artifact-dir target/pass4-public-ingress/phase4-fixture-20260511 \
  --run-id phase4-fixture-20260511
```

Expected artifacts:

- `public_endpoint_map.json`;
- `publisher_public_dht_snapshot.json`;
- `hostcreator_bootstrap_qr.png`;
- `hostcreator_bootstrap_seed.redacted.json`;
- public reachability transcript;
- admin-denial transcript;
- teardown transcript.

---

## Tests

Add focused tests for:

- endpoint descriptor validation rejects cluster-local DNS names, pod IPs, localhost, and
  admin listener ports;
- Publisher DHT signing uses public endpoint descriptors, not pod-internal addresses;
- HostCreator QR producer rejects missing public key, expired endpoint descriptors, and
  Publisher/bridge DHT shortcut fields;
- public ingress verifier fails if an admin endpoint is reachable;
- teardown invalidates or removes the temporary public endpoint map;
- Phase 1 strict bootstrap and SendDummy validations continue to pass before and after
  public exposure setup.

Run:

```bash
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
shellcheck infra/scripts/k8s-pass4-public-ingress-*.sh
```

---

## Acceptance Criteria

- A public endpoint map exists for local k8s Publisher, HostCreator bootstrap, and
  ExitBridge protocol surfaces.
- Endpoint descriptors contain only mobile-reachable public hosts/ports.
- Publisher-signed DHT entries use public endpoint descriptors for the validation run.
- `BootstrapDHTQRCode` contains HostCreator public key and reachability metadata, but no
  Publisher/bridge bootstrap shortcut data.
- Public verification proves required protocol endpoints are reachable from outside the
  cluster.
- Public verification proves admin endpoints are not reachable from outside the cluster.
- Phase 1 strict Bootstrap and SendDummy validations remain green.
- Teardown removes temporary exposure.
- V1 preservation checks return no files.
- Parent plan status tracker is updated.

---

## Completion Evidence

When this phase is implemented, archive:

- run-profile JSON;
- public endpoint map;
- public reachability transcript;
- admin-denial transcript;
- generated HostCreator QR and redacted seed payload;
- Publisher public-DHT snapshot with public endpoint descriptors;
- teardown transcript;
- Phase 1 strict validation output;
- V1 preservation command output.
