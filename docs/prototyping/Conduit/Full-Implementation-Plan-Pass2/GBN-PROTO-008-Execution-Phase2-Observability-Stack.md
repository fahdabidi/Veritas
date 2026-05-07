# GBN-PROTO-008 - Execution Phase 2 Detailed Plan: Observability Stack (Prometheus + Grafana + Loki + Promtail + Tempo)

**Status:** Implemented locally — live Helm install pending a WSL2 Docker/k3d session
**Primary Goal:** install a self-contained observability stack into the local k3d cluster
that provides metrics (Prometheus), dashboards + UI (Grafana), log aggregation (Loki +
Promtail), and distributed tracing (Tempo). Pre-provision Grafana datasources and a
Conduit overview dashboard. Configure Prometheus scraping for the Conduit pods that Phase
1 deployed, so the moment Phase 3 of this plan adds `/metrics` endpoints to the binaries,
data starts flowing.
**Source Plan:** [GBN-PROTO-008 Execution Plan](GBN-PROTO-008-Local-Kubernetes-Test-Infrastructure-Execution-Plan.md)
**AWS Equivalent:** GBN-PROTO-007 Phase 3 CloudWatch infrastructure (this phase produces
the local-k8s analog)

---

## 1. Current Repo Findings

| Item | Current Value | Why It Matters |
|---|---|---|
| Phase 1 deliverable | `infra/k8s/conduit/` manifests and k3d kubeconfig context | Phase 2 adds a sibling `infra/k8s/observability/` tree |
| Existing observability code in Conduit | none | Phase 2 only stands up the **stack**; Phase 3 wires Conduit binaries into it |
| Pod scrape annotations | added in Phase 1 (`prometheus.io/scrape: "true"` on each Conduit Deployment) | Prometheus needs configuration that honors these annotations |
| Helm | installed in Phase 1's bootstrap script | available for chart installs |

---

## 2. Review Summary

| Gap | Why It Matters | Resolution For Phase 2 |
|---|---|---|
| No metrics backend | Phase 3's emission has nowhere to go | install kube-prometheus-stack |
| No log aggregation | per-pod `kubectl logs` doesn't scale; no chain_id filter across pods | install loki-stack with Promtail |
| No tracing backend | GBN-PROTO-007 Phase 4's chain_id story has no visualization | install Tempo |
| No dashboards | operators get raw metric names without context | pre-provision a Conduit overview dashboard |
| No data flow until Phase 3 | the stack is ready but empty | document this as expected; Phase 3 lands data |

---

## 3. Scope Lock

### In Scope

- new directory `infra/k8s/observability/` containing Helm values overrides for each
  chart and any custom manifests
- Helm chart installs for:
  - `prometheus-community/kube-prometheus-stack` (Prometheus + Grafana + Alertmanager +
    node-exporter + kube-state-metrics)
  - `grafana/loki-stack` (Loki + Promtail; Grafana disabled because the prom stack
    already ships one)
  - `grafana/tempo` (Tempo single-binary mode; sufficient for local)
- pre-provisioned Grafana datasources for Prometheus, Loki, Tempo
- pre-provisioned Conduit overview dashboard JSON (one panel per metric family, plus a
  log panel and a trace search shortcut)
- `ServiceMonitor` (or pod-scrape config) for the Conduit pods so Prometheus discovers
  them automatically
- bring-up automation: `infra/scripts/k8s-observability-up.sh`
- tear-down: `infra/scripts/k8s-observability-down.sh`
- README section in [infra/README-infra.md](../../../prototype/gbn-bridge-proto/infra/README-infra.md)

### Out Of Scope

- alerting rules / Alertmanager routing (deferred — local cluster, no on-call)
- Grafana SSO (default admin/admin password for local)
- ingress / TLS (port-forward is fine locally)
- Tempo cluster mode (single-binary handles dev load)
- Adding metrics emission code to Conduit binaries (Phase 3 of this plan)
- Pre-provisioned alerting / SLO dashboards (deferred)

---

## 4. Preflight Gates

1. Phase 1 of GBN-PROTO-008 has landed; cluster `veritas` is up.
2. `helm version` works (installed by `bootstrap-k8s.sh`).
3. The `veritas` namespace is healthy (Conduit pods Running).
4. Outbound HTTPS to Helm chart repos.
5. V1 protected paths show no local diff.

---

## 5. File-by-File Specification

### 5.1 New file: `prototype/gbn-bridge-proto/infra/k8s/observability/values/kube-prometheus-stack.values.yaml`

```yaml
# Local-only sizing: minimal CPU/memory, short retention, default admin password.
fullnameOverride: kube-prom

prometheus:
  prometheusSpec:
    scrapeInterval: 15s
    evaluationInterval: 15s
    retention: 7d
    resources:
      requests: { cpu: 100m, memory: 256Mi }
      limits:   { cpu: 500m, memory: 1Gi }
    # Annotation-based pod discovery so manifest changes in infra/k8s/conduit/ are picked up.
    podMonitorSelectorNilUsesHelmValues: false
    serviceMonitorSelectorNilUsesHelmValues: false
    additionalScrapeConfigs:
      - job_name: conduit-pods
        kubernetes_sd_configs:
          - role: pod
            namespaces:
              names: [veritas]
        relabel_configs:
          - source_labels: [__meta_kubernetes_pod_annotation_prometheus_io_scrape]
            action: keep
            regex: "true"
          - source_labels: [__meta_kubernetes_pod_annotation_prometheus_io_path]
            target_label: __metrics_path__
            regex: (.+)
          - source_labels: [__address__, __meta_kubernetes_pod_annotation_prometheus_io_port]
            action: replace
            regex: ([^:]+)(?::\d+)?;(\d+)
            replacement: $1:$2
            target_label: __address__
          - source_labels: [__meta_kubernetes_pod_label_veritas_role]
            target_label: service
          - source_labels: [__meta_kubernetes_pod_name]
            target_label: pod

grafana:
  adminPassword: admin   # local only — note in README to never reuse
  defaultDashboardsTimezone: UTC
  service:
    type: NodePort
    nodePort: 30030
  additionalDataSources:
    - name: Loki
      type: loki
      url: http://loki.observability.svc.cluster.local:3100
      access: proxy
    - name: Tempo
      type: tempo
      url: http://tempo.observability.svc.cluster.local:3200
      access: proxy
      jsonData:
        tracesToLogs:
          datasourceUid: loki
          tags: [chain_id]
          mappedTags: [{ key: chain_id, value: chain_id }]
  dashboards:
    default:
      conduit-overview:
        json: |
          # see Section 5.4 — pre-provisioned dashboard JSON
  dashboardProviders:
    dashboardproviders.yaml:
      apiVersion: 1
      providers:
        - name: default
          orgId: 1
          folder: ''
          type: file
          disableDeletion: false
          editable: true
          options:
            path: /var/lib/grafana/dashboards/default

alertmanager:
  enabled: false   # not needed locally

kubeStateMetrics:
  enabled: true
nodeExporter:
  enabled: false   # noisy and not useful in k3d
```

### 5.2 New file: `prototype/gbn-bridge-proto/infra/k8s/observability/values/loki-stack.values.yaml`

```yaml
loki:
  enabled: true
  persistence:
    enabled: true
    size: 5Gi
    storageClassName: local-path
  config:
    limits_config:
      retention_period: 168h    # 7 days
    table_manager:
      retention_deletes_enabled: true
      retention_period: 168h

promtail:
  enabled: true
  config:
    snippets:
      pipelineStages:
        - cri: {}
        - regex:
            # Capture chain_id from structured log lines so Grafana → Tempo can jump.
            expression: '.*chain_id="(?P<chain_id>[^"]+)".*'
        - labels:
            chain_id:

grafana:
  enabled: false   # kube-prometheus-stack provides Grafana
prometheus:
  enabled: false   # likewise
```

### 5.3 New file: `prototype/gbn-bridge-proto/infra/k8s/observability/values/tempo.values.yaml`

```yaml
tempo:
  retention: 24h
  storage:
    trace:
      backend: local
      local:
        path: /var/tempo/traces

persistence:
  enabled: true
  size: 5Gi
  storageClassName: local-path

# OTLP receivers for Conduit binaries (Phase 3 will configure them to push here).
receivers:
  otlp:
    protocols:
      grpc:
        endpoint: 0.0.0.0:4317
      http:
        endpoint: 0.0.0.0:4318

service:
  type: ClusterIP
  ports:
    - { name: tempo-http,  port: 3200, targetPort: 3200 }
    - { name: otlp-grpc,   port: 4317, targetPort: 4317 }
    - { name: otlp-http,   port: 4318, targetPort: 4318 }
```

### 5.4 New file: `prototype/gbn-bridge-proto/infra/k8s/observability/dashboards/conduit-overview.json`

Pre-provisioned Grafana dashboard. JSON content (skeletal):

```json
{
  "title": "Conduit V2 Overview",
  "uid": "conduit-overview",
  "panels": [
    {
      "title": "Authority — Successful registrations",
      "type": "stat",
      "targets": [{ "expr": "sum(rate(conduit_authority_successful_registrations_total[5m]))" }]
    },
    {
      "title": "Authority — Bootstrap requests",
      "type": "graph",
      "targets": [{ "expr": "sum(rate(conduit_authority_bootstrap_requests_total[5m]))" }]
    },
    {
      "title": "Receiver — Frames accepted",
      "type": "graph",
      "targets": [{ "expr": "sum(rate(conduit_receiver_frames_accepted_total[5m]))" }]
    },
    {
      "title": "Bridge — Frames forwarded",
      "type": "graph",
      "targets": [{ "expr": "sum(rate(conduit_bridge_frames_forwarded_total[5m]))" }]
    },
    {
      "title": "Logs (filter by chain_id)",
      "type": "logs",
      "datasource": "Loki",
      "targets": [{ "expr": "{namespace=\"veritas\"} |= \"chain_id\"" }]
    }
  ],
  "templating": {
    "list": [
      {
        "name": "chain_id",
        "type": "textbox",
        "label": "chain_id filter",
        "current": { "value": "" }
      }
    ]
  }
}
```

The dashboard is mounted into Grafana via the `kube-prometheus-stack` values' `dashboards`
field (see §5.1) or via a sidecar ConfigMap with the `grafana_dashboard: "1"` label.

### 5.5 New file: `prototype/gbn-bridge-proto/infra/scripts/k8s-observability-up.sh`

```bash
#!/usr/bin/env bash
set -euo pipefail
NS="${VERITAS_OBS_NAMESPACE:-observability}"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
VALS="$ROOT_DIR/infra/k8s/observability/values"

helm repo add prometheus-community https://prometheus-community.github.io/helm-charts >/dev/null
helm repo add grafana https://grafana.github.io/helm-charts >/dev/null
helm repo update >/dev/null

kubectl create namespace "$NS" --dry-run=client -o yaml | kubectl apply -f -

helm upgrade --install kube-prom prometheus-community/kube-prometheus-stack \
  -n "$NS" -f "$VALS/kube-prometheus-stack.values.yaml" --wait

helm upgrade --install loki grafana/loki-stack \
  -n "$NS" -f "$VALS/loki-stack.values.yaml" --wait

helm upgrade --install tempo grafana/tempo \
  -n "$NS" -f "$VALS/tempo.values.yaml" --wait

# Apply the conduit dashboard ConfigMap if not bundled into Grafana values.
kubectl apply -f "$ROOT_DIR/infra/k8s/observability/dashboards/conduit-overview-cm.yaml"

echo ""
echo "Observability stack ready in namespace '$NS'."
echo "Grafana:   http://localhost:30030  (default user/pass: admin/admin)"
echo "Prometheus UI:  kubectl -n $NS port-forward svc/kube-prom-prometheus 9090:9090"
echo "Tempo UI in Grafana → Explore → Tempo datasource."
```

### 5.6 New file: `prototype/gbn-bridge-proto/infra/scripts/k8s-observability-down.sh`

```bash
#!/usr/bin/env bash
set -euo pipefail
NS="${VERITAS_OBS_NAMESPACE:-observability}"

read -r -p "Uninstall observability stack from namespace '$NS'? [y/N]: " confirm
[[ "${confirm,,}" != "y" ]] && exit 0

helm uninstall kube-prom -n "$NS" || true
helm uninstall loki      -n "$NS" || true
helm uninstall tempo     -n "$NS" || true

kubectl delete namespace "$NS" --ignore-not-found
echo "Observability stack removed."
```

### 5.7 New file: `prototype/gbn-bridge-proto/infra/k8s/observability/dashboards/conduit-overview-cm.yaml`

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: conduit-overview
  namespace: observability
  labels:
    grafana_dashboard: "1"
data:
  conduit-overview.json: |
    # contents of conduit-overview.json embedded here
```

(Use `kubectl create configmap conduit-overview --from-file=...json --dry-run=client -o
yaml` during implementation to generate the embedded version.)

### 5.8 Modify: `prototype/gbn-bridge-proto/infra/README-infra.md`

Add a section "Local Observability" pointing at:
- `bash infra/scripts/k8s-observability-up.sh`
- Grafana URL `http://localhost:30030` and default credentials
- the Conduit overview dashboard
- how to filter logs and traces by `chain_id`

---

## 6. Module And Asset Ownership Locked In Phase 2

| Asset | Responsibility |
|---|---|
| `infra/k8s/observability/values/*.yaml` | Helm values overrides |
| `infra/k8s/observability/dashboards/*.json` | pre-provisioned Grafana dashboards |
| `infra/k8s/observability/dashboards/*-cm.yaml` | ConfigMap wrappers for dashboard JSON |
| `infra/scripts/k8s-observability-up.sh` | one-shot install |
| `infra/scripts/k8s-observability-down.sh` | one-shot uninstall |

---

## 7. Implementation Notes

Phase 2 landed as local-only Helm values, dashboard provisioning, and install/remove
scripts:

1. `infra/k8s/observability/values/kube-prometheus-stack.values.yaml` installs
   Prometheus and Grafana with local resource limits, NodePort `30030`, annotation-based
   scraping for Conduit pods in the `veritas` namespace, and Loki/Tempo datasources.
2. `infra/k8s/observability/values/loki-stack.values.yaml` installs Loki and Promtail
   with 7-day retention and a Promtail pipeline that extracts `chain_id` labels from
   container logs when present.
3. `infra/k8s/observability/values/tempo.values.yaml` installs single-binary Tempo with
   local storage, 24-hour retention, and OTLP gRPC/HTTP receivers ready for Phase 3.
4. `infra/k8s/observability/dashboards/conduit-overview.json` and
   `conduit-overview-cm.yaml` pre-provision the `Conduit V2 Overview` dashboard through
   Grafana's dashboard sidecar.
5. `infra/scripts/k8s-observability-up.sh` adds Helm repos, installs Loki, Tempo, and
   kube-prometheus-stack into `observability`, applies the dashboard ConfigMap, and prints
   the Grafana/Prometheus/Tempo access URLs.
6. `infra/scripts/k8s-observability-down.sh` removes the Helm releases and namespace.
7. The Status Trackers table in
   [GBN-PROTO-008-Local-Kubernetes-Test-Infrastructure-Execution-Plan.md](GBN-PROTO-008-Local-Kubernetes-Test-Infrastructure-Execution-Plan.md)
   has been updated to mark Phase 2 complete.

Empty Prometheus panels were expected until Phase 3 added `/metrics` endpoints to the
Conduit binaries. After Phase 3 images are rebuilt/redeployed, Prometheus should scrape
authority `8080`, receiver `8081`, and bridge metrics `9100`.

---

## 8. Validation

Completed static/local validation in the current Windows-hosted shell:

1. `bash -n prototype/gbn-bridge-proto/infra/scripts/k8s-observability-up.sh prototype/gbn-bridge-proto/infra/scripts/k8s-observability-down.sh`
   passed.
2. PyYAML parsed every YAML file under `prototype/gbn-bridge-proto/infra/k8s/observability`.
3. Python `json` parsed `conduit-overview.json`.
4. The embedded `conduit-overview.json` content inside `conduit-overview-cm.yaml` parsed as JSON.
5. `git diff --check` passed with only Windows LF/CRLF warnings.
6. V1 protected-path diff was clean.

Deferred live WSL2 validation because this PowerShell environment does not have `docker`,
`k3d`, `kubectl`, or `helm` on PATH:

1. Phase 1's cluster is up.
2. Run `bash prototype/gbn-bridge-proto/infra/scripts/k8s-observability-up.sh`. Within
   ~5 minutes:
   - `kubectl -n observability get pods` shows Prometheus, Grafana, Loki, Promtail, Tempo
     all Running.
3. Open `http://localhost:30030`, log in `admin/admin`. The Conduit overview dashboard is
   visible under the default folder.
4. Datasource health check (Grafana → Connections → Data sources) shows Prometheus, Loki,
   Tempo all green.
5. If Phase 3 images have not been rebuilt/redeployed yet, empty data is expected.
   Confirm Prometheus has discovered the Conduit pod targets:
   `Status -> Targets` shows Conduit pod targets for the `conduit-pods` job. They may be
   DOWN with HTTP 404 scrape errors before Phase 3 deployment; after deployment they should
   be UP and return `conduit_*` series.
6. Promtail is shipping logs. Grafana → Explore → Loki, query
   `{namespace="veritas"}` — recent log lines from Conduit pods appear.
7. Tempo accepts OTLP (verify with the Tempo Service `kubectl -n observability port-forward
   svc/tempo 3200:3200` and `curl localhost:3200/ready` returns "ready"). Conduit trace
   data appears after Phase 3 images are rebuilt/redeployed with `GBN_BRIDGE_OTLP_ENDPOINT`.
8. Run `bash prototype/gbn-bridge-proto/infra/scripts/k8s-observability-down.sh`,
   confirm with `y`, namespace and Helm releases are removed.
9. Update this document with live Helm output once the WSL2 run completes.

---

## 9. Open Questions Carried Into Implementation

1. **Resource budget on a 8 GB WSL distro** — Prometheus is the heaviest component
   (~700 MB working set). If WSL has < 6 GB allocated to k3d, drop the kube-state-metrics
   and node-exporter scrapes to fit.
2. **Loki single-binary vs read/write split** — single-binary remains the local dev shape.
3. **Tempo OTLP gRPC vs HTTP** — Phase 3 of this plan will pick one. HTTP is simpler from
   Rust; gRPC has less per-span overhead. The Phase 2 Tempo values expose both.
4. **Default Grafana admin password** — `admin/admin`; README now warns that this is
   local-only.
5. **Dashboard JSON updates during iteration** — the ConfigMap-based load means a
   `kubectl apply` reloads the dashboard when JSON changes. Document this loop.
