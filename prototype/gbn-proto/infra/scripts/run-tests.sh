#!/usr/bin/env bash
# run-tests.sh — Execute the full Phase 1 Zero-Trust test suite on AWS.
#
# Usage: ./run-tests.sh <creator-ip> <publisher-ip> <relay1-ip> <relay2-ip> <relay3-ip> <relay4-ip> [ssh-key-path]
#
# This script:
#   1. Collects relay identity public keys from DHT-announced nodes
#   2. Starts the Publisher receiver
#   3. Runs the full 500MB pipeline (sanitize → chunk → encrypt → onion-route → reconstruct)
#   4. Triggers S1.9: kills a relay mid-transmission, validates recovery
#   5. Verifies SHA-256 integrity of the reassembled video
#   6. Reports timing metrics and pass/fail summary

set -euo pipefail

if [ "$#" -lt 6 ]; then
    echo "Usage: $0 <creator-ip> <publisher-ip> <relay1-ip> <relay2-ip> <relay3-ip> <relay4-ip> [ssh-key-path]"
    exit 1
fi

CREATOR_IP="$1"
PUBLISHER_IP="$2"
RELAY1_IP="$3"
RELAY2_IP="$4"
RELAY3_IP="$5"
RELAY4_IP="$6"
SSH_KEY="${7:-~/.ssh/gbn-proto-key.pem}"
SSH_USER="ec2-user"
REMOTE_DIR="/home/$SSH_USER/gbn-proto"

SSH_OPTS="-i $SSH_KEY -o StrictHostKeyChecking=no"
RESULTS_LOG="/tmp/gbn-phase1-results.log"

echo "============================================"
echo "  GBN Phase 1 — Zero-Trust Test Suite"
echo "  Creator:   $CREATOR_IP"
echo "  Publisher: $PUBLISHER_IP"
echo "  Relays:    $RELAY1_IP, $RELAY2_IP, $RELAY3_IP, $RELAY4_IP"
echo "============================================"
echo ""

# ─── Step 1: Collect relay identity public keys ──────────────────────────────
echo "[Step 1/6] Collecting relay identity public keys..."

RELAY_IPS=("$RELAY1_IP" "$RELAY2_IP" "$RELAY3_IP" "$RELAY4_IP")
RELAY_PUBKEYS=()

for IP in "${RELAY_IPS[@]}"; do
    PUB=$(ssh $SSH_OPTS "$SSH_USER@$IP" "cat $REMOTE_DIR/identity/identity.pub")
    RELAY_PUBKEYS+=("$PUB")
    echo "  $IP → ${PUB:0:16}..."
done

# Write keys to a topology file on the Creator so it can build circuits
ssh $SSH_OPTS "$SSH_USER@$CREATOR_IP" "mkdir -p $REMOTE_DIR/topology"
for i in "${!RELAY_IPS[@]}"; do
    echo "${RELAY_IPS[$i]}:$((9000 + i)) ${RELAY_PUBKEYS[$i]}" | \
    ssh $SSH_OPTS "$SSH_USER@$CREATOR_IP" \
        "cat >> $REMOTE_DIR/topology/relay-nodes.txt"
done
echo "  Topology written to Creator."

# ─── Step 2: Start Publisher receiver ────────────────────────────────────────
echo ""
echo "[Step 2/6] Starting publisher onion receiver..."

ssh $SSH_OPTS "$SSH_USER@$PUBLISHER_IP" \
    "nohup $REMOTE_DIR/gbn-proto receive \
        --listen-ports 9000,9001,9002 \
        --output-dir $REMOTE_DIR/reassembled/ \
        > /tmp/publisher.log 2>&1 &"
echo "  Publisher listening on ports 9000, 9001, 9002."
sleep 2

# ─── Step 3: Full pipeline (normal transmission) ─────────────────────────────
echo ""
echo "[Step 3/6] Running full 500MB pipeline with Telescopic Onion Routing..."

ssh $SSH_OPTS "$SSH_USER@$CREATOR_IP" \
    "$REMOTE_DIR/gbn-proto upload \
        --input $REMOTE_DIR/test-vectors/*.mp4 \
        --publisher-key $(ssh $SSH_OPTS "$SSH_USER@$PUBLISHER_IP" "cat $REMOTE_DIR/identity/identity.pub") \
        --relay-topology $REMOTE_DIR/topology/relay-nodes.txt \
        --dht-seed $RELAY1_IP:9100 \
        --paths 3 --hops 3 \
        2>&1" | tee "$RESULTS_LOG"

echo ""
echo "  ✅ Normal pipeline complete."

# ─── Step 4: S1.9 — Mid-Transmission Node Failure Test ───────────────────────
echo ""
echo "[Step 4/6] S1.9 — Simulating Guard node failure DURING transmission..."

# Reset Publisher for a fresh session
ssh $SSH_OPTS "$SSH_USER@$PUBLISHER_IP" "pkill -f 'gbn-proto receive' || true"
sleep 1
ssh $SSH_OPTS "$SSH_USER@$PUBLISHER_IP" \
    "nohup $REMOTE_DIR/gbn-proto receive \
        --listen-ports 9000,9001,9002 \
        --output-dir $REMOTE_DIR/reassembled-s19/ \
        > /tmp/publisher-s19.log 2>&1 &"
sleep 1

# Start a *background* upload — we will interrupt it mid-flight
ssh $SSH_OPTS "$SSH_USER@$CREATOR_IP" \
    "$REMOTE_DIR/gbn-proto upload \
        --input $REMOTE_DIR/test-vectors/*.mp4 \
        --publisher-key $(ssh $SSH_OPTS "$SSH_USER@$PUBLISHER_IP" "cat $REMOTE_DIR/identity/identity.pub") \
        --relay-topology $REMOTE_DIR/topology/relay-nodes.txt \
        --dht-seed $RELAY1_IP:9100 \
        --paths 3 --hops 3 \
        2>&1 &"

# Wait for transfer to be partially in-flight
echo "  Upload started. Waiting 15 seconds for partial transmission..."
sleep 15

# Kill Relay1 (the Guard for the first circuit) — simulates Spot instance preemption
echo "  Terminating Relay 1 ($RELAY1_IP) mid-transmission (S1.9)..."
ssh $SSH_OPTS "$SSH_USER@$RELAY1_IP" "pkill -f 'gbn-proto onion-relay' || true"

# Wait for the Creator's heartbeat to detect the failure and rebuild circuit
echo "  Waiting 20s for Circuit Manager heartbeat timeout and route rebuild..."
sleep 20

# Wait for upload to complete on the Creator
echo "  Waiting for Creator to complete re-routed transmission..."
wait 2>/dev/null || true
sleep 10

# ─── Step 5: Verify SHA-256 integrity ────────────────────────────────────────
echo ""
echo "[Step 5/6] Verifying SHA-256 integrity on Publisher..."

VERIFY_RESULT=$(ssh $SSH_OPTS "$SSH_USER@$PUBLISHER_IP" \
    "$REMOTE_DIR/gbn-proto verify \
        --reassembled $REMOTE_DIR/reassembled/*.mp4 \
        2>&1")
echo "$VERIFY_RESULT"

S19_RESULT=$(ssh $SSH_OPTS "$SSH_USER@$PUBLISHER_IP" \
    "$REMOTE_DIR/gbn-proto verify \
        --reassembled $REMOTE_DIR/reassembled-s19/*.mp4 \
        2>&1" || echo "S1.9 REASSEMBLY INCOMPLETE — FAIL")
echo "S1.9 result: $S19_RESULT"

# ─── Step 6: Cleanup ─────────────────────────────────────────────────────────
echo ""
echo "[Step 6/6] Stopping all remote processes..."

ALL_IPS=("$RELAY1_IP" "$RELAY2_IP" "$RELAY3_IP" "$RELAY4_IP" "$PUBLISHER_IP")
for IP in "${ALL_IPS[@]}"; do
    ssh $SSH_OPTS "$SSH_USER@$IP" "pkill -f gbn-proto || true" 2>/dev/null
done

echo ""
echo "============================================"
echo "  Phase 1 Test Suite Results"
echo "  Full log: $RESULTS_LOG"
echo "============================================"
echo ""
echo "$VERIFY_RESULT" | grep -q "PASS" && echo "✅ Normal pipeline: PASS" || echo "❌ Normal pipeline: FAIL"
echo "$S19_RESULT"    | grep -q "PASS" && echo "✅ S1.9 Node Recovery: PASS" || echo "❌ S1.9 Node Recovery: FAIL"
echo ""
echo "NEXT STEP: Run teardown.sh to destroy the CloudFormation stack and stop billing."
