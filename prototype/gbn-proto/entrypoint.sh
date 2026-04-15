#!/bin/bash
# entrypoint.sh — ECS container entrypoint wrapper.
# Injects GBN_INSTANCE_IPV4 from the ECS task metadata endpoint (awsvpc mode)
# and generates a per-container Noise_XX private key for onion circuit identity.
# Falls back gracefully outside ECS.
set -e

if [ -n "${ECS_CONTAINER_METADATA_URI_V4:-}" ]; then
  _meta="$(curl -sf "${ECS_CONTAINER_METADATA_URI_V4}")"
  export GBN_INSTANCE_IPV4
  GBN_INSTANCE_IPV4="$(echo "$_meta" | python3 -c \
    "import sys,json; d=json.load(sys.stdin); print(d['Networks'][0]['IPv4Addresses'][0])")"
fi

# Generate a per-container Noise_XX private key if not already injected.
# register_with_cloudmap() in swarm.rs derives the corresponding X25519 public
# key from this value and registers it so the Creator can discover this relay.
if [ -z "${GBN_NOISE_PRIVKEY_HEX:-}" ]; then
  export GBN_NOISE_PRIVKEY_HEX
  GBN_NOISE_PRIVKEY_HEX="$(openssl rand -hex 32)"
fi

exec "$@"
