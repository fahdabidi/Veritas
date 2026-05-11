# GBN-PROTO-013 Phase 1 Strict SendDummy Report

- Bootstrap session: `bootstrap-000007`
- Required route source: `local_dht`
- Normal ChainID: `smoke-3-normal-85634a2abeee4a3294322156a0ffc957`
- Normal bridge: `exit-bridge-1`
- Failover ChainID: `smoke-3-failover-9318839c1c16406b9dafadb3805f419e`
- Failover bridge: `exit-bridge-2`
- Ciphertext-only bridge check: `True`

## Prerequisite Bootstrap Privacy

| Gate | Status | Evidence |
|---|---:|---|
| CreatorBootstrap encrypted to NewCreator | `pass` | recipient_key_id=`new-creator`, ciphertext_len=`2684` |
| SeedBridgeCatalog encrypted to NewCreator | `pass` | recipient_key_id=`new-creator`, ciphertext_len=`12531` |
| HostCreator and ExitBridgeA cannot see bootstrap payload plaintext | `pass` | forbidden_plaintext_hits=`0` |

## DHT Evidence

- Publisher DHT entries: `10`
- NewCreator local DHT entries: `10`
- NewCreator active bridge entries: `10`
- NewCreator active tunnels: `10`
- Publisher encryption key present in NewCreator DHT: `True`
- NewCreator state: `onboarded`

## API Completion And Payload Validation

| Invocation | ChainID | Bridge | Route Source | Envelope | Frames | Payload Hash Match | Ciphertext Only At Bridge |
|---|---|---|---|---|---:|---:|---:|
| normal | smoke-3-normal-85634a2abeee4a3294322156a0ffc957 | exit-bridge-1 | local_dht | publisher_x25519_hkdf_aes256gcm_v1 | 1 | true | true |
| failover | smoke-3-failover-9318839c1c16406b9dafadb3805f419e | exit-bridge-2 | local_dht | publisher_x25519_hkdf_aes256gcm_v1 | 1 | true | true |

Artifacts: `bootstrap/strict-bootstrap-summary.json`, `bootstrap/bootstrap-relay-privacy-evidence.json`, `route/dht-evidence/pre-send/`, `route/send-dummy-*-result.json`, `route/received-dummy-*.json`, `route/bridge-plaintext-grep.txt`, `strict-senddummy-summary.json`, and `route/chainid-evidence/`.

Result: strict SendDummy validation passed.
