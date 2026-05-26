# GBN-PROTO-013 - Execution Phase 5 - Mobile To AWS Public Internet Validation

**Status:** Repo-side implementation complete; physical-phone AWS validation pending
**Last Updated:** 2026-05-12
**Parent Plan:** [GBN-PROTO-013](GBN-PROTO-013-Conduit-Mobile-Creator-Public-Internet-Validation-Execution-Plan.md)
**Depends On:** Phases 1-4 complete

> File-name note: this file keeps its original `Mobile-To-Local-K8s` path to avoid link
> churn in existing Pass 4 docs. The canonical Phase 5 validation target is now AWS, not
> local k8s.

## Objective

Run the Android creator app from a physical mobile device over a real mobile-network path
against an AWS-deployed Publisher, HostCreator, and ExitBridge topology.

The earlier local-k8s-public plan is no longer sufficient for Phase 5 sign-off. The
mobile path needs public, independently reachable protocol endpoints for the Publisher,
HostCreator bootstrap surface, and each ExitBridge. A local k8s cluster behind one
workstation/router cannot reliably provide that topology without NAT and endpoint
shortcuts that would weaken the validation. AWS is therefore the required Phase 5
validation environment.

Local k8s remains important, but only as a regression baseline:

- Phase 1 strict Bootstrap and SendDummy local-k8s gates must remain green.
- Pass 3 local-k8s smoke coverage must continue to pass.
- Local k8s public-ingress tooling from Phase 4 may be kept as a non-signoff fallback or
  fixture, but it is not the canonical Phase 5 mobile validation path.

At completion:

- AWS Publisher authority and receiver protocol endpoints are public and mobile-reachable;
- AWS HostCreator bootstrap endpoint is public and mobile-reachable;
- AWS ExitBridges have distinct public ingress/punch endpoints represented in signed DHT
  entries;
- admin, shell, database, metrics mutation, and Kubernetes/ECS control surfaces remain
  private;
- the Android app scans a real AWS-generated `BootstrapDHTQRCode`;
- mobile NewCreator starts with only HostCreator DHT seed material;
- Publisher public key/DHT and Seed ExitBridgeB DHT arrive in the encrypted Publisher
  bootstrap payload returned through HostCreator;
- mobile bootstrap reaches `onboarded` or a documented allowed partial terminal state;
- mobile `SendDummy` succeeds through a route selected from local mobile DHT;
- mobile full upload succeeds through AWS ExitBridge route/lane entries;
- forced failover/churn path produces evidence;
- mobile evidence is uploaded to S3 and retrieved on this workstation;
- AWS CloudWatch logs/traces correlate by ChainID.

Update the parent plan status tracker when this phase is complete.

---

## Required AWS Topology

Phase 5 requires a full AWS public validation topology:

```text
Android phone on cellular
  -> AWS HostCreator bootstrap public endpoint
  -> AWS Publisher authority/receiver public endpoints
  -> AWS ExitBridge public ingress/punch endpoints

AWS HostCreator
  -> relays NewCreator entry request to AWS Publisher through normal protocol path

AWS Publisher
  -> signs bootstrap payload and bridge catalog
  -> encrypts bootstrap payload to mobile NewCreator public key
  -> records authority/receiver evidence in CloudWatch

AWS ExitBridges
  -> receive Publisher-seeded DHT/catalog data
  -> establish tunnels/routes with mobile NewCreator
  -> record route/lane evidence in CloudWatch
```

Default cost-aware placement:

| Component | Default Region | Reason |
|---|---|---|
| Publisher authority/receiver | `us-east-1` | Low-cost baseline and common AWS service availability |
| HostCreator | `us-east-1` | Keeps first-contact/Publisher path simple for the baseline run |
| Seed ExitBridgeB | `ca-central-1` | Demonstrates non-U.S. bridge placement early |
| Remaining ExitBridges | `ca-central-1` | Keeps the bridge fleet in one non-U.S. region for cost and evidence clarity |
| S3 evidence bucket | `us-east-1` | Existing evidence bucket region |

The minimum live AWS run may start with three ExitBridges for cost control if the report
labels it `cost_minimum_aws_public_path`. A parity run with ten ExitBridges is required
before claiming full Pass 3 bridge-count parity.

---

## AWS Endpoint Rules

Every mobile-facing protocol actor must have a public endpoint descriptor that can be
placed into the Publisher-signed DHT/catalog flow.

Required endpoint roles:

- `publisher_authority`;
- `publisher_receiver`;
- `host_creator_bootstrap`;
- `exit_bridge` for each selected bridge.

Endpoint implementation choices are acceptable only when they preserve the protocol
contract:

- dedicated EC2 instance with Elastic IP per protocol actor;
- Network Load Balancer with static addresses and one listener/target group per actor;
- ECS/Fargate public task IPs only if the endpoint map is generated after tasks are
  running and descriptors are short-lived;
- no shared public endpoint that hides the selected bridge identity unless the mapping is
  explicit and traceable in the signed descriptor.

Forbidden:

- localhost, RFC1918/private, pod, task-private, or cluster-local addresses in
  mobile-facing descriptors;
- public admin HTTP endpoints;
- direct mobile access to `creator-runner` admin APIs;
- a preloaded Publisher DHT/trust ingest into the phone;
- using adb, kubectl, ECS Exec, or local files as the only evidence path.

---

## Preconditions

Required before running Phase 5:

- Phase 1 strict Bootstrap and SendDummy validations are green against local k8s.
- Phase 2 mobile FFI builds for `arm64-v8a`.
- Phase 3 Android debug APK installs and passes the manual device smoke.
- AWS account, region, IAM, VPC/subnet, security group, CloudWatch, and S3 evidence
  prerequisites are ready.
- AWS deployment plan creates Publisher, HostCreator, and ExitBridge protocol surfaces;
  it does not expose admin surfaces publicly.
- AWS endpoint map is generated from live AWS resources, not examples or placeholders.
- HostCreator QR seed is generated from the live AWS endpoint map.
- S3 evidence bucket and short-lived upload grant are prepared.
- Phone has cellular service; canonical run disables Wi-Fi.

The operator records the device model, Android version, ABI, carrier/network context, app
build id, Rust build id, AWS account id, region list, endpoint map id, and run id before
the live run starts.

---

## Repo-Side Implementation Status

The repo-side Phase 5 gaps are implemented. The remaining Phase 5 gate is the live
physical-phone validation against a deployed AWS topology using the runbook below.

### AWS Infrastructure Scripts

The Phase 5 AWS runbook uses these implemented scripts:

```text
prototype/gbn-bridge-proto/infra/scripts/aws-pass4-full-topology-plan.sh
prototype/gbn-bridge-proto/infra/scripts/aws-pass4-full-topology-prereqs.sh
prototype/gbn-bridge-proto/infra/scripts/aws-pass4-full-topology-up.sh
prototype/gbn-bridge-proto/infra/scripts/aws-pass4-full-topology-verify.sh
prototype/gbn-bridge-proto/infra/scripts/aws-pass4-full-topology-down.sh
prototype/gbn-bridge-proto/infra/scripts/aws-pass4-mobile-collector.sh
```

Those scripts reuse the existing AWS deployment model and add Phase 5 mobile-specific
artifact generation:

```text
prototype/gbn-bridge-proto/infra/scripts/mobile-validation-full.sh
prototype/gbn-bridge-proto/infra/scripts/collect-conduit-traces.sh
prototype/gbn-bridge-proto/infra/scripts/aws-smoke-creator-exec.sh
```

- deploy or discover AWS Publisher authority, Publisher receiver, HostCreator, and
  ExitBridge protocol endpoints;
- discover deploy prerequisites from AWS CLI-visible VPC, subnet, ECR, and Secrets
  Manager resources, while keeping explicit CLI/env overrides available;
- emit `target/pass4-aws-public/$RUN_ID/aws_public_endpoint_map.json`;
- emit `target/pass4-aws-public/$RUN_ID/run-profile.aws-public.live.json`;
- emit `infra/pass4/aws/run-profile.aws-public.live.json` for local inspection;
- emit `run_profile_qr.png` when `qrencode` is installed and always emit chunked
  `run_profile_qr_payloads/` for standalone phone import;
- emit `hostcreator_bootstrap_qr.png` when `qrencode` is installed and always emit
  `hostcreator_bootstrap_qr.svg`, `hostcreator_bootstrap_qr_payload.txt`, and
  `hostcreator_bootstrap_qr_payload.json`;
- verify public TCP/UDP reachability from outside private AWS paths;
- verify public denial for admin port `9090`;
- collect CloudWatch logs by ChainID for Publisher, Receiver, HostCreator, and selected
  ExitBridges;
- tear down every Phase 5 AWS resource or write an explicit deferred-teardown note.

### AWS Profile Artifacts

The AWS pass4 profile template is present:

```text
prototype/gbn-bridge-proto/infra/pass4/aws/run-profile.aws-public.template.json
```

The live run creates:

```text
prototype/gbn-bridge-proto/infra/pass4/aws/run-profile.aws-public.live.json
```

The live profile must use `profile=aws_public` and must not contain Publisher DHT,
Publisher public key, Seed ExitBridge DHT, bridge catalog, or any private/admin endpoint
preload. Those values must be learned through the encrypted bootstrap/catalog flow.

### Android Code Changes

The Android validation app now supports:

- `RunProfileConfig` adds `PROFILE_AWS_PUBLIC = "aws_public"` and allows it in
  `allowedProfiles`.
- `MainActivity` shows `aws_public` in the Network Profile selector.
- `RunProfileQRReader` supports raw and chunked AWS run-profile QR payloads.
- `Import Run Profile Document` supports standalone-phone file import.
- `Evidence Grant QR Reader` and `Import S3 Grant Document` support standalone-phone S3
  evidence grant import.
- `phase5PrerequisiteError(...)` permits `aws_public` for
  `BootstrapNewCreator`, `SendDummy`, and `SendUpload`.
- The default run-profile text area has an AWS public template when
  `aws_public` is selected.
- The evidence bundle includes the imported AWS run profile, AWS endpoint map id, and
  CloudWatch query hints.
- Android unit/instrumentation tests assert `aws_public` import and Phase 5 button
  availability.

### Rust/Mobile Runtime Changes

The Rust/mobile runtime now accepts the `aws_public` profile and enables the Phase 5
creator operations against the imported live endpoint descriptors:

- `bootstrapNewCreator` requires an imported HostCreator DHT seed and an AWS public
  endpoint profile, then persists Publisher and ExitBridge DHT entries derived from the
  live AWS descriptors.
- `sendDummy` dispatches through an active ExitBridge route selected from mobile local
  DHT.
- `sendUpload` dispatches upload lanes/chunks over active ExitBridge entries.
- `exportEvidence` includes operation results, selected AWS bridge ids, ChainIDs,
  DHT snapshots, and CloudWatch query hints.
- `creator-runner` retains local-only admin binding and adds a separate non-admin
  HostCreator bootstrap hint endpoint for AWS Phase 5.
- Existing `creator-runner` HTTP/admin APIs remain backward compatible with Pass 3.

---

## Gap Closure Workstreams

The `2026-05-12` Phase 5 attempt proved that the local-k8s strict prerequisites and the
Android/emulator Phase 3 path are useful, but it also exposed a topology issue: the live
phone path should not depend on mapping many independent public node identities through a
single local k8s workstation. Phase 5 must close the AWS deployment and evidence gaps
before the physical-phone smoke can be signed off.

### 1. AWS Public Validation Profile

Create a live AWS profile, for example:

```text
prototype/gbn-bridge-proto/infra/pass4/aws/run-profile.aws-public.live.json
```

The profile must contain live AWS endpoint descriptors for:

- Publisher authority HTTPS/TLS endpoint;
- Publisher receiver HTTPS/TLS endpoint;
- HostCreator bootstrap HTTPS/TLS endpoint;
- each ExitBridge TCP/UDP endpoint;
- admin-denial URLs or private-only evidence for every admin surface that must remain
  unreachable from public internet.

Required profile fields:

- profile name: `aws_public`;
- AWS account id and run id;
- public host/IP, protocol, and port for each endpoint;
- TLS SNI and certificate fingerprint for every HTTPS/TLS endpoint;
- AWS region and availability zone where meaningful;
- endpoint expiry timestamp;
- HostCreator actor id and public key;
- bridge actor ids and public endpoint identity metadata;
- shared run `chain_id`;
- no localhost, RFC1918/private, cluster-local, task-private, pod DNS, or admin endpoint
  shortcuts in mobile-facing DHT descriptors.

The live run must verify DNS, TCP/TLS, UDP where required, and admin-denial evidence from
outside AWS-private paths before the phone starts.

### 2. AWS Deployment Tooling

Add or adapt AWS operator tooling for the full Phase 5 topology:

```text
prototype/gbn-bridge-proto/infra/scripts/aws-pass4-full-topology-plan.sh
prototype/gbn-bridge-proto/infra/scripts/aws-pass4-full-topology-prereqs.sh
prototype/gbn-bridge-proto/infra/scripts/aws-pass4-full-topology-up.sh
prototype/gbn-bridge-proto/infra/scripts/aws-pass4-full-topology-verify.sh
prototype/gbn-bridge-proto/infra/scripts/aws-pass4-full-topology-down.sh
prototype/gbn-bridge-proto/infra/scripts/aws-pass4-mobile-collector.sh
```

Expected behavior:

- guard for WSL2 Ubuntu;
- require explicit AWS region input and record AWS account identity;
- discover VPC/subnet/ECR/Secrets prerequisites from AWS CLI-visible resources, with
  explicit CLI/env overrides for any ambiguous value;
- print estimated cost/resource count before deploy;
- deploy Publisher authority/receiver, HostCreator, and ExitBridge services/tasks;
- assign or discover stable public endpoints per protocol actor;
- keep admin surfaces private through security groups/IAM/ECS Exec only;
- generate `aws_public_endpoint_map.json`;
- generate `run-profile.aws-public.live.json`;
- generate `run_profile_qr.png` or chunked run-profile QR payloads for standalone phone
  import;
- generate `hostcreator_bootstrap_qr.png`, `.svg`, and payload text from AWS
  HostCreator metadata;
- verify public DNS/TLS/UDP reachability from the workstation path;
- verify public admin denial;
- write CloudWatch log group/stream hints for every actor;
- tear down all AWS resources after evidence is archived.

### 3. Mobile FFI Public Operations

The Phase 5 mobile runtime methods remain required:

- `bootstrapNewCreator`
- `sendDummy`
- `sendUpload`

They must use the same strict rules as the hardened flow:

- `bootstrapNewCreator` starts with only the imported AWS HostCreator DHT seed and the
  mobile NewCreator's DHT entry/public key.
- Publisher public key, Publisher DHT, and Seed ExitBridgeB DHT are accepted only from
  the Publisher bootstrap payload encrypted to the mobile NewCreator.
- HostCreator and relay bridge transit data remains opaque; no plaintext bootstrap
  payload is exposed to transit actors.
- `sendDummy` routes only from the mobile local DHT and returns route source, bridge id,
  result state, payload/hash evidence, and ChainID.
- `sendUpload` dispatches the selected upload session through active mobile local-DHT
  bridge entries and records lane/chunk ACK evidence.
- all network operations are asynchronous or event-emitting from the Kotlin caller's
  perspective and preserve ChainID.

The FFI implementation must keep the existing `creator-runner` HTTP/admin APIs intact so
Pass 3 scripts and reports continue to work.

### 4. Android Phase 5 Controls

The Android app must support the AWS validation profile.

Required app changes or configuration:

- add/select `aws_public` network profile;
- import `run-profile.aws-public.live.json` through `RunProfileQRReader`, document picker,
  or share intent; adb-only import is a lab fallback and not valid for standalone phone
  sign-off;
- gate `BootstrapNewCreator`, `SendDummy`, and `SendUpload` on runtime/profile/seed
  state/onboarding/session state;
- canonical run has Wi-Fi disabled and cellular/mobile path available;
- HostCreator seed was imported from an AWS-generated QR payload that passed public-key,
  reachability, expiry, payload-hash, and no-Publisher-preload checks;
- disabled states must show the exact missing prerequisite.

The app must display and export results for bootstrap, SendDummy, upload, failover,
local-DHT snapshots, ChainID events, AWS endpoint map id, CloudWatch query hints, and S3
upload metadata.

### 5. AWS Phase 5 Collector

Implemented:

```text
prototype/gbn-bridge-proto/infra/scripts/aws-pass4-mobile-collector.sh
```

The collector must:

- accept `--run-id`, `--stack-name`, `--region`, one or more `--chain-id`, optional
  `--mobile-evidence-s3-uri`, and `--require-chain-id`;
- fetch the mobile evidence ZIP from S3;
- verify downloaded ZIP SHA-256 against the app-side manifest;
- unpack and validate required files, including `local_dht.json`,
  `host_creator_seed.redacted.json`, `trace_events.jsonl`, `remote_trace_queries.json`,
  `network_context.json`, and `manifest.sha256.json`;
- collect CloudWatch logs from Publisher authority, Publisher receiver, HostCreator, and
  selected ExitBridges;
- query AWS observability surfaces for every ChainID;
- attach AWS endpoint map, HostCreator QR manifest, admin-denial transcript, and S3
  retrieval transcript;
- fail if required ChainIDs, local-DHT route evidence, encrypted bootstrap evidence,
  CloudWatch evidence, or no-public-admin evidence is missing.

The collector output becomes the canonical Phase 5 report input.

### 6. Standalone S3 Grant Import

The physical phone is standalone in Phase 5. It is not connected through adb and the
pre-signed S3 `PUT` URL is too long and error-prone to type manually. Phase 5 therefore
requires a QR-based evidence grant handoff.

Required app control:

```text
EvidenceGrantQRReader
```

The canonical grant import path is:

1. The workstation generates a short-lived S3 `PUT` grant JSON.
2. The workstation renders the grant as one or more QR payloads.
3. The phone scans the QR payloads from the workstation screen.
4. The app reconstructs and validates the complete grant.
5. The app stores the grant in app-private state and uses it for evidence upload.

Because AWS pre-signed URLs can be larger than a single reliable QR payload, the QR format
must support chunking:

```json
{
  "type": "gbn.s3_grant.chunk",
  "version": 1,
  "grant_id": "pass4-phase5-20260512T201424Z",
  "index": 1,
  "count": 3,
  "sha256": "hex-sha256-of-complete-grant-json",
  "data": "base64url(json-chunk)"
}
```

Rules:

- `index` is 1-based;
- all chunks must share `grant_id`, `count`, and `sha256`;
- duplicate chunks are accepted only if their payload is byte-identical;
- reconstructed grant JSON SHA-256 must match `sha256`;
- `expires_at_ms` must be in the future;
- the app must reject grants that contain long-lived AWS credentials or raw
  `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, or session-token fields outside the
  pre-signed URL;
- the app must show scan progress, final bucket/object key, and expiry before upload.

Fallbacks:

- Android document picker import from Downloads/Files;
- Android share intent into the app;
- adb file import for emulator/lab only.

---

## Mobile Run Flow

### Bootstrap

1. Install the debug APK produced by Phase 3.
2. Disable Wi-Fi on the phone.
3. Launch the app.
4. Select `aws_public`.
5. Import the live AWS public run profile.
6. Start runtime.
7. Scan the AWS HostCreator QR with `HostCreatorDHTQRReader`.
8. Confirm HostCreator id, public-key fingerprint, AWS public endpoint, region, expiry,
   payload hash, and ChainID.
9. Import the seed.
10. Tap `BootstrapNewCreator`.
11. Wait for terminal state.

Expected:

- QR import inserts HostCreator seed only.
- Mobile NewCreator sends its own DHT and public key to AWS HostCreator.
- HostCreator relays the request to AWS Publisher through the existing path.
- Publisher encrypts the bootstrap payload to the mobile NewCreator public key.
- HostCreator and relay bridge forward opaque bytes only.
- NewCreator learns Publisher public key/DHT and Seed ExitBridgeB DHT from the encrypted
  payload.
- Seed ExitBridgeB returns the signed bridge catalog after tunnel establishment.
- Remaining ExitBridges establish tunnels before being marked active.

### SendDummy

1. Open Creator Actions.
2. Tap `DumpLocalDht`.
3. Tap `SendDummy`.
4. Accept or enter a dummy payload size.
5. Confirm ChainID.

Expected:

- route source is `local_dht`;
- selected bridge is an AWS ExitBridge, active, non-expired, and mobile-reachable;
- bridge sees ciphertext only;
- AWS Publisher receiver accepts the frame;
- app shows assigned bridge id, result, and ChainID;
- CloudWatch logs contain matching ChainID evidence.

### Full Upload

1. Build a synthetic upload session or choose a small local test file.
2. Verify manifest/content hash and chunk count.
3. Tap `SendUpload`.
4. Use default lane count first.
5. Wait for upload completion and receiver content-hash match.

Expected:

- chunks are encrypted before crossing any bridge;
- dispatch plan uses active AWS bridge entries learned through mobile local DHT;
- receiver reconstructs content with matching hash;
- progressive fanout evidence records lane open and chunk ACK events.

### Failover / Churn

Run one of:

- `SendDummy` with forced bridge-failure option;
- `SendUpload` with forced lane failure;
- operator-side temporary disable of one AWS ExitBridge public endpoint/task.

Expected:

- affected bridge is marked suspect or failed;
- route/lane is reselected from mobile local DHT;
- operation completes or records explicit degraded terminal state;
- ChainID evidence includes the failover decision in mobile evidence and CloudWatch.

---

## Evidence Transfer

The phone exports evidence through the app, uploads it to S3, and this workstation
retrieves it through AWS APIs.

Required workstation setup creates a short-lived S3 `PUT` grant JSON for the app and then
renders it as chunked QR payloads. Do not embed AWS credentials in the APK. Use an
operator-side helper such as boto3
`generate_presigned_url(ClientMethod="put_object", HttpMethod="PUT")`:

```json
{
  "upload_mode": "s3_presigned_put",
  "bucket": "veritas-pass4-mobile-evidence",
  "object_key": "mobile-evidence/<run_id>/<chain_id>/<bundle_id>.zip",
  "presigned_put_url": "https://...",
  "expires_at_ms": 1770000000000
}
```

Then generate the QR handoff:

```bash
infra/scripts/pass4-s3-grant-qr.sh \
  --grant-json /tmp/pass4-s3-grant.json \
  --out-dir target/pass4-s3-grants/$RUN_ID
```

The app imports the grant through `EvidenceGrantQRReader`. The document/share/adb import
paths are fallback paths only; QR import is the canonical standalone-phone path.

Workstation retrieval:

```bash
aws s3 cp \
  s3://veritas-pass4-mobile-evidence/mobile-evidence/<run_id>/<chain_id>/<bundle_id>.zip \
  /tmp/veritas-pass4-mobile-evidence/<bundle_id>.zip
```

The report must prove that the S3-downloaded bundle hash matches the app-side manifest.

---

## End-To-End Operator Runbook

This is the step-by-step command flow for the physical Android phone validation. Run all
workstation commands from WSL2 Ubuntu unless explicitly noted.

### Step 0 - Set Run Variables

```bash
cd /mnt/c/Users/fahd_/OneDrive/Documents/Veritas/prototype/gbn-bridge-proto

export RUN_ID="pass4-phase5-aws-$(date -u +%Y%m%dT%H%M%SZ)"
export AWS_REGION_PHASE5="ca-central-1"
export STACK_NAME="gbn-conduit-full-pass4"
export ENVIRONMENT_NAME="pass4"
export BRIDGE_COUNT="3"
export BRIDGE_UDP_PORT="4443"
export AWS_PROFILE_CONFIG="infra/pass4/aws/run-profile.aws-public.live.json"
export AWS_ARTIFACT_DIR="target/pass4-aws-public/$RUN_ID"
export EVIDENCE_BUCKET="veritas-pass4-mobile-evidence"
export EVIDENCE_PREFIX="mobile-evidence/$RUN_ID"

mkdir -p "$AWS_ARTIFACT_DIR"
printf '%s\n' "$RUN_ID" | tee "$AWS_ARTIFACT_DIR/run_id.txt"
```

The Phase 5 AWS helper attempts to discover deployment prerequisites from AWS after the
supporting account resources exist:

- default VPC, public service subnets, and database subnets;
- latest ECR images from `gbn-conduit-full-authority`, `gbn-conduit-full-receiver`,
  `gbn-conduit-full-bridge`, and `gbn-conduit-full-creator`;
- likely Publisher signing-key and bridge signing-seed Secret ARNs from Secrets Manager;
- Publisher public key from a readable `publisher_public_key_hex`/`public_key_hex`
  field, a dedicated public-key secret, or optional local Ed25519 derivation if Python
  `cryptography` is installed and the signing seed is readable.

If discovery picks the wrong resource, override only that value with an environment
variable or the matching `aws-pass4-full-topology-up.sh` argument:

```bash
export VPC_ID="<vpc-id>"
export SERVICE_SUBNET_IDS="<subnet-a>,<subnet-b>"
export DATABASE_SUBNET_IDS="<db-subnet-a>,<db-subnet-b>"
export AUTHORITY_IMAGE="<authority-image-uri>"
export RECEIVER_IMAGE="<receiver-image-uri>"
export BRIDGE_IMAGE="<bridge-image-uri>"
export CREATOR_IMAGE="<creator-image-uri>"
export PUBLISHER_SIGNING_KEY_SECRET_ARN="<publisher-signing-key-secret-arn>"
export BRIDGE_SIGNING_SEED_SECRET_ARN="<bridge-signing-seed-secret-arn>"
export PUBLISHER_PUBLIC_KEY_HEX="<64-hex-char-publisher-public-key>"
```

Confirm AWS identity before provisioning:

```bash
aws sts get-caller-identity | tee "$AWS_ARTIFACT_DIR/aws-caller-identity.json"
```

### Step 1 - Run Local Regression Gates

These do not prove the mobile path. They prove the hardened local baseline still works
before AWS/mobile validation begins.

```bash
infra/scripts/k8s-smoke-bootstrap-strict-v4.sh --require-observability \
  | tee "$AWS_ARTIFACT_DIR/local-bootstrap-regression.log"

infra/scripts/k8s-smoke-senddummy-strict-v4.sh --require-observability \
  | tee "$AWS_ARTIFACT_DIR/local-senddummy-regression.log"
```

### Step 2 - Build And Smoke The Android APK

```bash
cd /mnt/c/Users/fahd_/OneDrive/Documents/Veritas/prototype/gbn-bridge-proto/mobile/android

./gradlew test lint assembleDebug \
  | tee ../../target/pass4-aws-public/$RUN_ID/android-build.log

./gradlew connectedDebugAndroidTest \
  | tee ../../target/pass4-aws-public/$RUN_ID/android-connected-test.log

cd /mnt/c/Users/fahd_/OneDrive/Documents/Veritas/prototype/gbn-bridge-proto
```

APK path:

```text
prototype/gbn-bridge-proto/mobile/android/app/build/outputs/apk/debug/app-debug.apk
```

Install with adb when the phone is authorized:

```bash
adb devices
adb -s <PHONE_SERIAL> install -r mobile/android/app/build/outputs/apk/debug/app-debug.apk
```

If adb does not see the physical phone, use MTP/manual install:

```bash
explorer.exe "$(wslpath -w mobile/android/app/build/outputs/apk/debug)"
```

Then copy `app-debug.apk` to the phone `Downloads` folder and install it from Android
Files/My Files.

### Step 3 - Review The AWS Public Profile Template

The live profile is generated from deployed AWS task public IPs by Step 5. Review the
template only to confirm that it contains no bootstrap/DHT preload fields:

```bash
mkdir -p infra/pass4/aws
cat infra/pass4/aws/run-profile.aws-public.template.json
```

The generated live profile must contain:

- `"profile": "aws_public"`;
- AWS account id;
- Publisher authority/receiver endpoint descriptors;
- HostCreator bootstrap endpoint descriptor;
- ExitBridge endpoint descriptors;
- evidence bucket/prefix;
- no Publisher DHT, Publisher public key, Seed ExitBridge DHT, bridge catalog, private
  address, or admin endpoint preload.

Phase 5 uses UDP `4443` for ExitBridge public ingress by default. The original AWS
shape used UDP `443`, but the bridge container runs as the non-root `veritas` user and
Fargate did not honor file capabilities for binding a privileged UDP port. The
validation requirement is a real public mobile-network path, not a specific privileged
port, so the stack, security group, generated mobile profile, and verifier must all use
`BRIDGE_UDP_PORT`.

### Step 4 - Plan AWS Resources And Discover Deploy Prerequisites

```bash
infra/scripts/aws-pass4-full-topology-plan.sh \
  --run-id "$RUN_ID" \
  --stack-name "$STACK_NAME" \
  --region "$AWS_REGION_PHASE5" \
  --bridge-count "$BRIDGE_COUNT" \
  --discover-prereqs \
  --artifact-dir "$AWS_ARTIFACT_DIR" \
  | tee "$AWS_ARTIFACT_DIR/aws-plan.log"

infra/scripts/aws-pass4-full-topology-prereqs.sh \
  --run-id "$RUN_ID" \
  --stack-name "$STACK_NAME" \
  --region "$AWS_REGION_PHASE5" \
  --artifact-dir "$AWS_ARTIFACT_DIR" \
  | tee "$AWS_ARTIFACT_DIR/aws-prereqs.log"

cat "$AWS_ARTIFACT_DIR/aws-deploy-prerequisites.json"
```

Review the plan and prerequisite output before continuing. The plan must show:

- Publisher authority/receiver resources;
- HostCreator resource;
- `BRIDGE_COUNT` ExitBridge resources;
- public protocol ports only;
- private admin access only;
- CloudWatch log groups/streams;
- estimated cost/resource count.

The prerequisite output must show `"ok": true`. If any item is listed in `missing`, either
create the missing AWS prerequisite or export the override value shown in Step 0 before
running Step 5.

### Step 5 - Deploy AWS Public Topology

```bash
infra/scripts/aws-pass4-full-topology-up.sh \
  --run-id "$RUN_ID" \
  --stack-name "$STACK_NAME" \
  --region "$AWS_REGION_PHASE5" \
  --environment "$ENVIRONMENT_NAME" \
  --bridge-count "$BRIDGE_COUNT" \
  --bridge-udp-port "$BRIDGE_UDP_PORT" \
  --artifact-dir "$AWS_ARTIFACT_DIR" \
  --evidence-bucket "$EVIDENCE_BUCKET" \
  --evidence-prefix "$EVIDENCE_PREFIX" \
  | tee "$AWS_ARTIFACT_DIR/aws-up.log"
```

The deploy helper resolves VPC/subnets/ECR images/Secrets/public key using this order:
explicit CLI argument, environment variable, then AWS discovery. It writes the exact
resolved values and sources to:

```text
target/pass4-aws-public/$RUN_ID/aws-deploy-prerequisites.json
```

If the stack is already deployed and you only need to regenerate the live profile/QR
artifacts from current task public IPs, use:

```bash
infra/scripts/aws-pass4-full-topology-up.sh \
  --discover-existing \
  --run-id "$RUN_ID" \
  --stack-name "$STACK_NAME" \
  --region "$AWS_REGION_PHASE5" \
  --bridge-count "$BRIDGE_COUNT" \
  --bridge-udp-port "$BRIDGE_UDP_PORT" \
  --artifact-dir "$AWS_ARTIFACT_DIR" \
  | tee "$AWS_ARTIFACT_DIR/aws-discover.log"
```

Expected artifacts after deploy:

```text
target/pass4-aws-public/$RUN_ID/aws_public_endpoint_map.json
target/pass4-aws-public/$RUN_ID/run-profile.aws-public.live.json
target/pass4-aws-public/$RUN_ID/hostcreator_bootstrap_qr.png
target/pass4-aws-public/$RUN_ID/hostcreator_bootstrap_qr_payload.txt
target/pass4-aws-public/$RUN_ID/run_profile_qr_payloads/
```

Validate the generated live profile:

```bash
rg -n "localhost|127\\.0\\.0\\.1|10\\.|172\\.(1[6-9]|2[0-9]|3[0-1])\\.|192\\.168\\.|cluster\\.local|\\.svc|publisher_entry|publisher_dht|publisher_public_key|seed_exit_bridge|bridge_catalog|admin_url" \
  "$AWS_ARTIFACT_DIR/run-profile.aws-public.live.json" && {
    echo "ERROR: AWS profile contains forbidden bootstrap/private/admin material" >&2
    exit 1
  }
```

Expected: no `rg` matches.

### Step 6 - Verify AWS Public Reachability And Admin Denial

```bash
infra/scripts/aws-pass4-full-topology-verify.sh \
  --run-id "$RUN_ID" \
  --stack-name "$STACK_NAME" \
  --region "$AWS_REGION_PHASE5" \
  --bridge-udp-port "$BRIDGE_UDP_PORT" \
  --artifact-dir "$AWS_ARTIFACT_DIR" \
  | tee "$AWS_ARTIFACT_DIR/aws-verify.log"
```

The verify step must fail if public endpoint descriptors contain private addresses, if
admin ports are reachable, if DNS/TLS/UDP checks fail, or if CloudWatch hints are
missing.

Display the AWS HostCreator QR and run-profile import QR on the workstation:

```bash
explorer.exe "$(wslpath -w "$AWS_ARTIFACT_DIR")"
```

Open these files for phone scanning:

```text
hostcreator_bootstrap_qr.png
run_profile_qr.png
```

If `run_profile_qr.png` is chunked, scan every QR payload under:

```text
run_profile_qr_payloads/
```

### Step 7 - Prepare S3 Evidence Upload Grant

Create a short-lived pre-signed PUT grant:

```bash
python3 -m venv target/aws-presign-venv
source target/aws-presign-venv/bin/activate
python -m pip install -q boto3 "botocore[crt]"

export EVIDENCE_OBJECT_KEY="$EVIDENCE_PREFIX/mobile-bundle.zip"
export S3_REGION="us-east-1"

python - <<'PY' > /tmp/pass4-s3-grant.json
import json, os, time
import boto3

bucket = os.environ["EVIDENCE_BUCKET"]
key = os.environ["EVIDENCE_OBJECT_KEY"]
region = os.environ.get("S3_REGION", "us-east-1")
expires = 3600

url = boto3.client("s3", region_name=region).generate_presigned_url(
    ClientMethod="put_object",
    Params={"Bucket": bucket, "Key": key, "ContentType": "application/zip"},
    ExpiresIn=expires,
    HttpMethod="PUT",
)
print(json.dumps({
    "upload_mode": "s3_presigned_put",
    "bucket": bucket,
    "object_key": key,
    "presigned_put_url": url,
    "expires_at_ms": int((time.time() + expires) * 1000),
}, indent=2))
PY
```

Render the S3 grant as QR payloads:

```bash
infra/scripts/pass4-s3-grant-qr.sh \
  --grant-json /tmp/pass4-s3-grant.json \
  --out-dir "target/pass4-s3-grants/$RUN_ID" \
  | tee "$AWS_ARTIFACT_DIR/s3-grant-qr.log"

explorer.exe "$(wslpath -w "target/pass4-s3-grants/$RUN_ID")"
```

### Step 8 - Run The Phone Validation

On the physical Android phone:

1. Disable Wi-Fi.
2. Confirm mobile/cellular data is active.
3. Launch `GBN Mobile Creator`.
4. Select `aws_public`.
5. Import the AWS run profile with `RunProfileQRReader` or document/share fallback.
6. Tap `Start Runtime`.
7. Scan `hostcreator_bootstrap_qr.png` with `HostCreatorDHTQRReader`.
8. Confirm HostCreator id, public-key fingerprint, AWS endpoint, region, expiry, payload
   hash, and ChainID.
9. Tap `Import Host Seed`.
10. Tap `BootstrapNewCreator`.
11. Save the bootstrap ChainID shown in the app.
12. Tap `DumpLocalDht`; confirm Publisher and Seed ExitBridge entries are present.
13. Tap `SendDummy`; save the SendDummy ChainID.
14. Tap `Build Synthetic Upload Session`.
15. Tap `SendUpload`; save the upload ChainID.
16. Trigger forced bridge/lane failure through the AWS operator script or planned manual
   bridge task stop.
17. Run `SendDummy` or `SendUpload` again; save the failover ChainID.
18. Scan every S3 evidence grant QR chunk with `EvidenceGrantQRReader`.
19. Tap `Export Evidence`.
20. Tap `Upload Evidence To S3`.
21. Record the displayed S3 object key, ETag, bundle SHA-256, and file count.

Record the ChainIDs in WSL:

```bash
cat > "$AWS_ARTIFACT_DIR/mobile-chainids.env" <<'EOF'
BOOTSTRAP_CHAIN_ID=<from-phone>
SENDDUMMY_CHAIN_ID=<from-phone>
UPLOAD_CHAIN_ID=<from-phone>
FAILOVER_CHAIN_ID=<from-phone>
EOF
```

### Step 9 - Retrieve Mobile Evidence From S3

```bash
source "$AWS_ARTIFACT_DIR/mobile-chainids.env"

mkdir -p "$AWS_ARTIFACT_DIR/s3"
aws s3 cp \
  "s3://$EVIDENCE_BUCKET/$EVIDENCE_OBJECT_KEY" \
  "$AWS_ARTIFACT_DIR/s3/mobile-bundle.zip" \
  | tee "$AWS_ARTIFACT_DIR/s3-retrieval.log"

sha256sum "$AWS_ARTIFACT_DIR/s3/mobile-bundle.zip" \
  | tee "$AWS_ARTIFACT_DIR/s3/mobile-bundle.sha256"
```

### Step 10 - Collect AWS CloudWatch And Correlate ChainIDs

```bash
infra/scripts/aws-pass4-mobile-collector.sh \
  --run-id "$RUN_ID" \
  --stack-name "$STACK_NAME" \
  --region "$AWS_REGION_PHASE5" \
  --chain-id "$BOOTSTRAP_CHAIN_ID" \
  --chain-id "$SENDDUMMY_CHAIN_ID" \
  --chain-id "$UPLOAD_CHAIN_ID" \
  --chain-id "$FAILOVER_CHAIN_ID" \
  --mobile-evidence-s3-uri "s3://$EVIDENCE_BUCKET/$EVIDENCE_OBJECT_KEY" \
  --artifact-dir "$AWS_ARTIFACT_DIR/collector" \
  --require-chain-id \
  | tee "$AWS_ARTIFACT_DIR/aws-mobile-collector.log"
```

Expected collector output:

```text
target/pass4-aws-public/$RUN_ID/collector/aws-mobile-collection.json
```

### Step 11 - Archive Phase 5 Report Inputs

```bash
mkdir -p docs/prototyping/Conduit/Full-Implementation-Plan-Pass4/Test-Reports/artifacts/$RUN_ID

cp -R "$AWS_ARTIFACT_DIR" \
  "docs/prototyping/Conduit/Full-Implementation-Plan-Pass4/Test-Reports/artifacts/$RUN_ID/"
```

Create the Phase 5 report only after the collector passes:

```text
docs/prototyping/Conduit/Full-Implementation-Plan-Pass4/Test-Reports/GBN-PROTO-013-Phase5-Mobile-AWS-Public-Validation-<date>.md
```

### Step 12 - Tear Down AWS Resources

Always run teardown after evidence is collected, unless the report explicitly documents
an owner, reason, and cleanup date for keeping resources alive.

```bash
infra/scripts/aws-pass4-full-topology-down.sh \
  --run-id "$RUN_ID" \
  --stack-name "$STACK_NAME" \
  --region "$AWS_REGION_PHASE5" \
  --artifact-dir "$AWS_ARTIFACT_DIR" \
  | tee "$AWS_ARTIFACT_DIR/aws-down.log"
```

Verify no Phase 5 public resources remain:

```bash
aws cloudformation describe-stacks \
  --region "$AWS_REGION_PHASE5" \
  --stack-name "$STACK_NAME" \
  >/tmp/pass4-stack-after-down.json 2>/tmp/pass4-stack-after-down.err && {
    echo "ERROR: stack still exists after teardown" >&2
    cat /tmp/pass4-stack-after-down.json
    exit 1
  }

cat /tmp/pass4-stack-after-down.err | tee "$AWS_ARTIFACT_DIR/aws-down-verify.log"
```

### Step 13 - V1 Preservation Check

```bash
git diff --stat -- prototype/gbn-proto/
git diff --stat -- docs/prototyping/Lattice/
```

Expected: no output.

---

## Validation

Run from WSL2 Ubuntu:

```bash
uname -a | grep -i microsoft >/dev/null || { echo "Pass 4 tooling requires WSL2 Ubuntu" >&2; exit 1; }

cd prototype/gbn-bridge-proto

RUN_ID="pass4-phase5-aws-$(date -u +%Y%m%dT%H%M%SZ)"
STACK_NAME="gbn-conduit-full-pass4"
AWS_REGION_PHASE5="ca-central-1"

infra/scripts/aws-pass4-full-topology-plan.sh \
  --run-id "$RUN_ID" \
  --stack-name "$STACK_NAME" \
  --region "$AWS_REGION_PHASE5" \
  --bridge-count 3 \
  --discover-prereqs \
  --artifact-dir "target/pass4-aws-public/$RUN_ID"

infra/scripts/aws-pass4-full-topology-prereqs.sh \
  --run-id "$RUN_ID" \
  --stack-name "$STACK_NAME" \
  --region "$AWS_REGION_PHASE5" \
  --artifact-dir "target/pass4-aws-public/$RUN_ID"

infra/scripts/aws-pass4-full-topology-up.sh \
  --run-id "$RUN_ID" \
  --stack-name "$STACK_NAME" \
  --region "$AWS_REGION_PHASE5" \
  --bridge-count 3 \
  --artifact-dir "target/pass4-aws-public/$RUN_ID"

infra/scripts/aws-pass4-full-topology-verify.sh \
  --run-id "$RUN_ID" \
  --stack-name "$STACK_NAME" \
  --region "$AWS_REGION_PHASE5" \
  --artifact-dir "target/pass4-aws-public/$RUN_ID"

infra/scripts/aws-pass4-mobile-collector.sh \
  --run-id "$RUN_ID" \
  --stack-name "$STACK_NAME" \
  --region "$AWS_REGION_PHASE5" \
  --chain-id <bootstrap_chain_id> \
  --chain-id <senddummy_chain_id> \
  --chain-id <upload_chain_id> \
  --chain-id <failover_chain_id> \
  --mobile-evidence-s3-uri s3://veritas-pass4-mobile-evidence/mobile-evidence/$RUN_ID/mobile-bundle.zip \
  --require-chain-id
```

The collector must gather:

- mobile evidence bundle from S3;
- AWS Publisher authority CloudWatch logs;
- AWS Publisher receiver CloudWatch logs;
- AWS HostCreator CloudWatch logs;
- AWS ExitBridge CloudWatch logs for selected routes;
- AWS endpoint map and HostCreator QR manifest;
- no-public-admin evidence.

After the live run, tear down the AWS topology:

```bash
infra/scripts/aws-pass4-full-topology-down.sh \
  --run-id "$RUN_ID" \
  --stack-name "$STACK_NAME" \
  --region "$AWS_REGION_PHASE5" \
  --artifact-dir "target/pass4-aws-public/$RUN_ID"
```

---

## Tests

Add tests for:

- AWS public profile validation rejects example profiles, unresolved DNS, private hosts,
  cluster-local names, admin ports, and expired descriptors;
- AWS deployment plan excludes public admin listeners;
- app supports `aws_public` without preloading Publisher/bridge bootstrap state;
- app refuses bootstrap when Wi-Fi/cellular requirement is not satisfied for canonical
  validation mode;
- app refuses `BootstrapNewCreator` before HostCreator seed import;
- app enables `BootstrapNewCreator`, `SendDummy`, and `SendUpload` only when their Phase 5
  prerequisites are satisfied;
- AWS HostCreator QR payload rejects Publisher/bridge DHT preload;
- bootstrap accepts Publisher public key/DHT only from encrypted payload;
- `bootstrapNewCreator`, `sendDummy`, and `sendUpload` no longer return
  `not_implemented` for valid Phase 5 requests;
- SendDummy route selection uses mobile local DHT;
- upload session dispatch uses active mobile local DHT entries;
- evidence bundle contains mobile, AWS endpoint, DHT, app build, Rust build, and remote
  query files;
- S3 retrieval hash matches local evidence manifest;
- AWS collector fails if any required ChainID is missing;
- AWS collector fails if CloudWatch evidence is absent for selected actors;
- S3 grant chunk reconstruction accepts out-of-order QR chunks and rejects hash mismatch,
  duplicate mismatch, expired grants, and grants carrying raw long-lived AWS credentials.

Run:

```bash
cd prototype/gbn-bridge-proto
cargo fmt --all --check
cargo check --workspace
cargo test --workspace

cd mobile/android
./gradlew test
./gradlew connectedDebugAndroidTest
```

---

## Acceptance Criteria

- AWS public topology deploys Publisher, HostCreator, and ExitBridges.
- AWS endpoint map exists and contains mobile-reachable public endpoints for every
  required protocol actor.
- Public endpoints are unique or explicitly mapped per actor so selected bridge identity
  is traceable.
- Public admin access is denied.
- Android `EvidenceGrantQRReader` can import a chunked S3 grant without adb or typing.
- Mobile FFI implements `bootstrapNewCreator`, `sendDummy`, and `sendUpload`.
- Android `aws_public`, `BootstrapNewCreator`, `SendDummy`, and `SendUpload` controls are
  enabled only by their strict Phase 5 prerequisites.
- Physical phone validation runs with Wi-Fi disabled for the canonical run.
- Android app scans a real HostCreator QR generated from AWS public endpoint data.
- Mobile bootstrap uses no separate Publisher ingest and no private admin endpoint.
- Mobile local DHT contains Publisher and Seed ExitBridgeB entries learned from the
  encrypted Publisher bootstrap payload.
- Mobile local DHT contains bridge catalog entries learned through Seed ExitBridgeB.
- `SendDummy` succeeds with `route_source=local_dht`.
- Full upload completes with content hash match.
- Forced failure reroutes or records an explicit degraded state with ChainID evidence.
- Mobile evidence is uploaded to S3 and retrieved on this workstation.
- AWS CloudWatch logs/traces correlate to mobile ChainIDs.
- AWS teardown succeeds after validation.
- V1 preservation checks return no files.
- Parent plan status tracker is updated.

---

## Completion Evidence

When this phase is complete, archive:

- Android app build id and Rust build id;
- physical device/network context;
- live AWS public run profile used for the run;
- AWS endpoint map;
- AWS deployment plan output and cost/resource summary;
- HostCreator QR seed and redacted manifest;
- QR scan/import screenshots or instrumentation captures;
- mobile evidence ZIP from S3;
- S3 retrieval transcript and hash verification;
- S3 grant QR payloads/PNGs and app-side grant import evidence;
- AWS CloudWatch trace/log bundle;
- bootstrap report;
- SendDummy report;
- upload report;
- failover/churn report;
- collector transcript and generated report;
- AWS teardown transcript;
- V1 preservation command output.
