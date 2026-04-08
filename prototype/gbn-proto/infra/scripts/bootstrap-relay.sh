#!/usr/bin/env bash
# bootstrap-relay.sh — Run on each EC2 relay instance via user-data or SSH.
#
# Generates a permanent Ed25519 identity keypair for this relay node and
# writes the public key to a known path. The public key must be read and
# passed to the Creator's Circuit Manager so it can validate Noise_XX
# handshakes. The private key never leaves this instance.
#
# Usage (run as ec2-user):
#   ./bootstrap-relay.sh

set -euo pipefail

REMOTE_DIR="/home/ec2-user/gbn-proto"
KEY_DIR="$REMOTE_DIR/identity"

mkdir -p "$KEY_DIR"

if [ -f "$KEY_DIR/identity.key" ]; then
    echo "Identity keypair already exists — skipping generation."
else
    echo "Generating relay identity keypair..."
    # Use the gbn-proto binary to generate a keypair (writes identity.key + identity.pub)
    "$REMOTE_DIR/gbn-proto" keygen --out-dir "$KEY_DIR"
    echo "✅ Keypair generated:"
    echo "   Private: $KEY_DIR/identity.key  (never leaves this instance)"
    echo "   Public:  $KEY_DIR/identity.pub  (must be shared with Creator out-of-band)"
fi

echo ""
echo "Relay public key (share with Creator):"
cat "$KEY_DIR/identity.pub"
