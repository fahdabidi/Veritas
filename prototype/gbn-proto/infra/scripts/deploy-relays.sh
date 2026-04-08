#!/usr/bin/env bash
# deploy-relays.sh — Deploy the relay binary to all Relay EC2 instances.
#
# Usage: ./deploy-relays.sh <relay1-ip> <relay2-ip> <relay3-ip> <relay4-ip> <dht-seed-ip> [ssh-key-path]
#
# Each relay is started in 'onion-relay' mode. It:
#   - Generates (or loads) a persistent Ed25519 identity keypair
#   - Announces its signed RelayDescriptor to the Kademlia DHT
#   - Listens for telescopic Noise_XX connections from the Creator

set -euo pipefail

if [ "$#" -lt 5 ]; then
    echo "Usage: $0 <relay1-ip> <relay2-ip> <relay3-ip> <relay4-ip> <dht-seed-ip> [ssh-key-path]"
    exit 1
fi

RELAY1_IP="$1"
RELAY2_IP="$2"
RELAY3_IP="$3"
RELAY4_IP="$4"
DHT_SEED_IP="$5"          # First relay bootstraps the DHT; others peer off it
SSH_KEY="${6:-~/.ssh/gbn-proto-key.pem}"
SSH_USER="ec2-user"
REMOTE_DIR="/home/$SSH_USER/gbn-proto"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROTO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BINARY="$PROTO_ROOT/target/x86_64-unknown-linux-gnu/release/gbn-proto"

echo "============================================"
echo "  GBN Phase 1 — Deploy Relays"
echo "============================================"

if [ ! -f "$BINARY" ]; then
    echo "ERROR: Binary not found at $BINARY"
    echo "Run deploy-creator.sh first (it builds the binary)."
    exit 1
fi

RELAY_IPS=("$RELAY1_IP" "$RELAY2_IP" "$RELAY3_IP" "$RELAY4_IP")

for i in "${!RELAY_IPS[@]}"; do
    IP="${RELAY_IPS[$i]}"
    NUM=$((i + 1))
    PORT=$((9000 + i))
    echo "[Relay $NUM] Deploying to $IP (port $PORT)..."

    ssh -i "$SSH_KEY" -o StrictHostKeyChecking=no "$SSH_USER@$IP" \
        "mkdir -p $REMOTE_DIR"

    # Upload binary
    scp -i "$SSH_KEY" -o StrictHostKeyChecking=no \
        "$BINARY" \
        "$SSH_USER@$IP:$REMOTE_DIR/"

    # Upload and run bootstrap to generate identity keypair
    scp -i "$SSH_KEY" -o StrictHostKeyChecking=no \
        "$(dirname "$0")/bootstrap-relay.sh" \
        "$SSH_USER@$IP:$REMOTE_DIR/"
    ssh -i "$SSH_KEY" -o StrictHostKeyChecking=no "$SSH_USER@$IP" \
        "bash $REMOTE_DIR/bootstrap-relay.sh"

    # Start onion relay: loads identity.key, joins DHT via seed, listens on port
    SEED_ARG=""
    if [ "$IP" != "$DHT_SEED_IP" ]; then
        SEED_ARG="--dht-seed $DHT_SEED_IP:9100"
    fi

    ssh -i "$SSH_KEY" -o StrictHostKeyChecking=no "$SSH_USER@$IP" \
        "nohup $REMOTE_DIR/gbn-proto onion-relay \
            --identity $REMOTE_DIR/identity/identity.key \
            --listen 0.0.0.0:$PORT \
            --dht-listen 0.0.0.0:9100 \
            $SEED_ARG \
            > /tmp/relay-$PORT.log 2>&1 &"

    echo "[Relay $NUM] ✅ Started on port $PORT (DHT on 9100)."
done

echo ""
echo "✅ All 4 relay instances deployed as Onion Relays."
echo "   Relay public keys are in /home/ec2-user/gbn-proto/identity/identity.pub on each host."
echo "   Collect them and pass to the Creator for circuit validation."
