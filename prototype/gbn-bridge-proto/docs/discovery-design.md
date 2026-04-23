# Conduit Weak Discovery Design

Phase 7 adds weak discovery as a creator-local hint layer only.

Trust rules:

- publisher-signed bootstrap entries remain the highest-priority refresh candidates
- publisher-signed catalog descriptors remain transport-authoritative
- weak discovery hints can seed later catalog refresh attempts
- weak discovery hints cannot make a bridge transport-eligible by themselves
- weak discovery cannot replace the publisher-selected seed bridge or initial bridge set for first-contact bootstrap

Merge precedence:

1. active publisher-seeded bootstrap entries
2. freshest signed catalog descriptors
3. weak discovery hints

Implementation boundary:

- `discovery.rs` stores and filters weak hints
- `seed_catalog.rs` provides deterministic static seeds
- `hint_merge.rs` performs precedence and dedupe
- creator upload and transport selection continue to use signed state only
