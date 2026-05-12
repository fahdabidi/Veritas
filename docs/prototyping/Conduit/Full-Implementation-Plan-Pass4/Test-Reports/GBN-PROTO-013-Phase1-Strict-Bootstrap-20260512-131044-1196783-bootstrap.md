# GBN-PROTO-013 Phase 1 Strict Bootstrap Report

- ChainID: `smoke-2-d2669cc97f6e45e5aa1f638338493de7`
- Bootstrap session: `bootstrap-000009`
- Seed bridge: `exit-bridge-4`
- Bridge count: `10`
- Per-bridge fanout progress events: `10`
- Initial plaintext bridge set present: `False`
- Publisher DHT entry in encrypted bootstrap payload: `True`
- CreatorBootstrap ciphertext bytes: `2703`
- SeedBridgeCatalog ciphertext bytes: `12515`
- Transit actor bootstrap plaintext hits: `0`

## Payload Encryption And Relay Privacy

| Payload | Protection Gate | Status | Evidence |
|---|---|---:|---|
| CreatorBootstrap | encrypted to NewCreator; no initial plaintext bridge set | `pass` | recipient_key_id=`new-creator`, ciphertext_len=`2703`, initial_plaintext_bridge_set_present=`False` |
| SeedBridgeCatalog | encrypted to NewCreator before catalog handoff | `pass` | recipient_key_id=`new-creator`, ciphertext_len=`12515` |
| Relay transit visibility | HostCreator and ExitBridgeA current-chain logs contain no bootstrap-payload fields | `pass` | relay_bridge_id=`exit-bridge-0`, forbidden_plaintext_hits=`0` |

## README Flow Gate Ledger

| Step | Required flow gate | Status | Evidence artifact | Observed |
|---:|---|---:|---|---|
| 1 | NewCreator pairs with HostCreator | `pass` | `seed-new-creator-payload.json, seed-new-creator-result.json` | new_creator=new-creator host_creator=host-creator |
| 2 | NewCreator sends DHT entry and public key to HostCreator | `pass` | `seed-new-creator-result.json, bootstrap-session.json` | creator_dht_entry=new-creator encryption_key_present=True |
| 3 | HostCreator relays entry request through existing bridge path | `pass` | `pod-logs/*.log` | host_creator_join_relayed_via_bridge and publisher_join_received observed |
| 4 | Publisher creates signed bootstrap payload with NewCreator, Publisher, and Seed ExitBridgeB DHT | `pass` | `seed-new-creator-result.json, local-dht-final.json, bootstrap-session.json` | publisher_entry=publisher seed_bridge=exit-bridge-4 |
| 5 | Publisher encrypts bootstrap payload to NewCreator public key | `pass` | `strict-bootstrap-summary.json` | ciphertext_len=2703 |
| 6 | Publisher seeds ExitBridgeB with remaining bridge DHT set | `pass` | `bootstrap-session.json` | seed_payload_reporter=exit-bridge-4 seed_catalog_bridge_count=10 |
| 7 | Encrypted bootstrap payload returns through Publisher -> ExitBridgeA -> HostCreator -> NewCreator | `pass` | `pod-logs/*.log, bootstrap-relay-privacy-evidence.json` | publisher_response_to_host_via_bridge, host_relayed_response_to_new_creator, and new_creator_bootstrap_response_received observed; transit_plaintext_hits=0 |
| 8 | NewCreator decrypts payload and stores Publisher + Seed ExitBridgeB DHT state | `pass` | `local-dht-final.json, strict-bootstrap-summary.json` | publisher_entry_present=True seed_bridge=exit-bridge-4 |
| 9 | NewCreator and ExitBridgeB establish seed tunnel and report progress | `pass` | `bootstrap-session.json` | seed_tunnel_reporters=['exit-bridge-4', 'new-creator'] |
| 10 | NewCreator requests bridge catalog from ExitBridgeB | `pass` | `pod-logs/*.log` | new_creator_bridge_set_requested observed |
| 11 | ExitBridgeB returns signed remaining bridge catalog | `pass` | `seed-new-creator-result.json, pod-logs/*.log` | seed_bridge_bridge_set_returned observed; catalog_bridge_count=10 |
| 12 | Publisher fans out NewCreator DHT to remaining ExitBridges | `pass` | `pod-logs/*.log` | publisher_remaining_bridges_triggered observed |
| 13 | Remaining ExitBridges establish tunnels with NewCreator and report progress | `pass` | `bootstrap-session.json` | bridge_tunnel_established=10/10 |
| 14 | NewCreator marks each bridge active only after corresponding progress | `pass` | `local-dht-final.json, bootstrap-session.json` | active_bridge_count=10 progress_bridge_count=10 |
| 15 | Every step preserves the same ChainID | `pass` | `seed-new-creator-result.json, bootstrap-session.json, local-dht-final.json, pod-logs/*.log` | chain_id=smoke-2-d2669cc97f6e45e5aa1f638338493de7 |

Artifacts: `seed-new-creator-result.json`, `bootstrap-session.json`, `local-dht-final.json`, `strict-bootstrap-summary.json`, `strict-bootstrap-flow-steps.json`, `bootstrap-relay-privacy-evidence.json`, `pod-log-events.json`, and `pod-logs/`.

Result: strict bootstrap hardening validation passed.
