# GBN-PROTO-013 - Execution Phase 9 - Reports, Operators, And Acceptance

**Status:** Pending
**Last Updated:** 2026-05-11
**Parent Plan:** [GBN-PROTO-013](GBN-PROTO-013-Conduit-Mobile-Creator-Public-Internet-Validation-Execution-Plan.md)
**Depends On:** Phases 1-8 complete

## Objective

Close Pass 4 by archiving reports, updating operator documentation, updating
`README-infra.md`, and proving the remaining mobile-network validation gap is resolved.

This phase is documentation and acceptance only. It must not paper over missing evidence:
if any Phase 1-8 gate is incomplete, the README remains unchanged and the final report
records the blocker.

Update the parent plan status tracker when this phase is complete.

---

## Required Reports

Create Pass 4 reports under:

```text
docs/prototyping/Conduit/Full-Implementation-Plan-Pass4/Test-Reports/
```

Required report set:

| Report | Purpose |
|---|---|
| Bootstrap Hardening Report | Phase 1 strict Bootstrap and SendDummy local-k8s evidence |
| Mobile Runtime Report | Phase 2 FFI/library build, tests, and compatibility evidence |
| Android App Report | Phase 3 app build, device smoke, button parity, S3 evidence export |
| Local Public Exposure Report | Phase 4 public endpoint map, reachability, admin denial, teardown |
| Mobile Local k8s Report | Phase 5 phone-over-cellular bootstrap, SendDummy, upload, failover |
| Hybrid AWS Topology Report | Phase 6 bridge-only plan and security/cost review |
| AWS Bridge Deployment Report | Phase 7 `ca-central-1` bridge deployment and Publisher catalog evidence |
| Mobile AWS Geo Report | Phase 8 same-app mobile validation with AWS bridges and CloudWatch evidence |
| Final Acceptance Report | Cross-report index, ChainID matrix, README update decision |

Every report must include artifact paths, ChainIDs, command transcripts, status, and any
residual risk.

---

## ChainID Matrix

The final acceptance report must include a ChainID matrix:

| Flow | ChainID | Mobile Evidence | Local k8s Evidence | AWS Evidence | Result |
|---|---|---|---|---|---|
| Strict local bootstrap | required | N/A | required | N/A | pass/fail |
| Strict local SendDummy | required | N/A | required | N/A | pass/fail |
| Mobile local bootstrap | required | required | required | N/A | pass/fail |
| Mobile local SendDummy | required | required | required | N/A | pass/fail |
| Mobile local upload | required | required | required | N/A | pass/fail |
| Mobile local failover | required | required | required | N/A | pass/fail |
| Mobile hybrid bootstrap | required | required | required | required if AWS bridge selected | pass/fail |
| Mobile hybrid SendDummy | required | required | required | required | pass/fail |
| Mobile hybrid upload | required | required | required | required | pass/fail |
| Mobile hybrid failover | required | required | required | required | pass/fail |

No row can be marked pass without artifact references.

---

## Operator Documentation Updates

Update or add operator docs for:

- WSL2 Ubuntu prerequisites;
- Android app build/install;
- S3 evidence bucket setup and short-lived upload grants;
- AWS public topology deployment and teardown;
- AWS HostCreator `BootstrapDHTQRCode` generation;
- physical phone validation checklist;
- mobile evidence retrieval from S3;
- CloudWatch trace collection for AWS Publisher, HostCreator, and ExitBridges;
- restoring private-only AWS admin access after validation.

The docs must clearly distinguish operator/admin tooling from mobile app buttons. The
mobile app must never be described as calling private admin URLs from the phone.

---

## README Update Rule

Only update `prototype/gbn-bridge-proto/infra/README-infra.md` after:

1. Phase 1 strict Bootstrap validation passes.
2. Phase 1 strict SendDummy validation passes.
3. Phase 5 physical mobile AWS public validation passes.
4. Phase 8 physical mobile AWS geo validation passes, or Phase 5 includes the accepted
   non-U.S. ExitBridge evidence.
5. Reports are archived under Pass 4 `Test-Reports/`.
6. AWS resources are torn down or explicitly documented as intentionally still running.
7. V1 preservation check returns no files.

The README update must replace the remaining validation gap with a short evidence summary
and links to the Pass 4 reports.

---

## Final Acceptance Validation

Run from WSL2 Ubuntu:

```bash
uname -a | grep -i microsoft >/dev/null || { echo "Pass 4 tooling requires WSL2 Ubuntu" >&2; exit 1; }

# V1 untouched
git diff --stat -- prototype/gbn-proto/
git diff --stat -- docs/prototyping/Lattice/

cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
infra/scripts/k8s-pass3-acceptance.sh --require-observability
infra/scripts/k8s-smoke-bootstrap-strict-v4.sh --require-observability
infra/scripts/k8s-smoke-senddummy-strict-v4.sh --require-observability

cd mobile/android
./gradlew test
./gradlew lint
./gradlew assembleDebug
./gradlew connectedDebugAndroidTest
```

Final report validation:

```bash
cd ../../..
prototype/gbn-bridge-proto/infra/scripts/pass4-report-index-verify.sh \
  --require-chainid-matrix \
  --require-s3-mobile-evidence \
  --require-cloudwatch-evidence \
  --require-readme-links
```

---

## Tests

Add focused tests for:

- report index verifier fails when required report files are missing;
- report index verifier fails when ChainID matrix rows lack artifacts;
- README update checker fails if the validation gap is removed without report links;
- S3 evidence references include bucket, key, ETag or hash, and retrieval transcript;
- CloudWatch evidence references include region, log group, stream, ChainID, and bridge id;
- V1 preservation checker fails if V1 or Lattice files are modified.

Run:

```bash
cd prototype/gbn-bridge-proto
shellcheck infra/scripts/pass4-report-index-verify.sh
```

---

## Acceptance Criteria

- All Phase 1-8 acceptance criteria are complete or explicitly deferred with README gap
  left open.
- Required Pass 4 reports exist under `Test-Reports/`.
- Final ChainID matrix links mobile, S3, and AWS CloudWatch evidence.
- README remaining validation gap is updated only when evidence supports it.
- Operator docs describe the mobile app, S3 evidence transfer, AWS public topology, and
  CloudWatch collection.
- AWS resources are torn down or documented with owner/reason.
- Any local k8s public exposure used as a fallback fixture is torn down.
- Existing Pass 3 acceptance remains green.
- Rust and Android validation commands pass.
- V1 preservation checks return no files.
- Parent plan status tracker is updated.

---

## Completion Evidence

When this phase is implemented, archive:

- report index;
- final acceptance report;
- ChainID matrix;
- README diff;
- operator doc diff;
- final Rust/Android/test transcripts;
- Pass 3 regression transcript;
- S3 evidence retrieval transcript;
- local public ingress teardown transcript;
- AWS teardown transcript;
- V1 preservation command output.
