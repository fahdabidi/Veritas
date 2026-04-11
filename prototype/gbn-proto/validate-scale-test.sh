#!/bin/bash
set -euo pipefail

# Validation script for GBN Phase 1 Scale Test (Step 5 Local Validation)
# This script should be run after `docker-compose -f docker-compose.scale-test.yml up -d`
# It validates the four local pass criteria:
# 1. PlumTree gossip convergence within 15 seconds
# 2. Speculative dialing: Creator builds 3+ disjoint circuits
# 3. Geofence filter: Exit hops are exclusively relay-free containers
# 4. Circuit rebuild: Kill a relay mid-transfer, verify chunk re-queued

cd "$(dirname "$0")"

RUST_LOG="${RUST_LOG:-info,mcn_router_sim=debug}"
export RUST_LOG

echo "🔍 Starting GBN Phase 1 Local Validation (Step 5)"

# 1. Check that all services are running
echo "📋 Checking Docker Compose services..."
SERVICES=$(docker-compose -f docker-compose.scale-test.yml ps --services)
for svc in $SERVICES; do
    count=$(docker-compose -f docker-compose.scale-test.yml ps -q "$svc" | wc -l)
    expected=1
    if [[ "$svc" == "relay-hostile" ]]; then
        expected=18
    elif [[ "$svc" == "relay-free" ]]; then
        expected=2
    fi
    if [[ $count -lt $expected ]]; then
        echo "❌ Service $svc has only $count containers (expected $expected)"
        exit 1
    else
        echo "✅ Service $svc: $count/$expected containers"
    fi
done

# 2. Wait for bootstrap and gossip convergence (simplified check)
echo "⏳ Waiting 15 seconds for gossip convergence..."
sleep 15

# 3. Run a simple test using the proto-cli to verify circuit building
echo "🔧 Building test binary..."
cargo build --release --bin gbn-proto 2>&1 | tail -20

# 4. Generate publisher keys if they don't exist
if [[ ! -f publisher.key ]] || [[ ! -f publisher.pub ]]; then
    echo "🔑 Generating publisher keypair..."
    ./target/release/gbn-proto keygen
fi

# 5. Run a small upload test (using test video if exists, otherwise create dummy)
TEST_VIDEO="test-video.mp4"
if [[ ! -f "$TEST_VIDEO" ]]; then
    echo "📹 Creating dummy test video (1MB)..."
    dd if=/dev/zero of="$TEST_VIDEO" bs=1M count=1 2>/dev/null
fi

echo "🚀 Starting small-scale upload test (1 chunk, 3 paths)..."
# We'll run the upload with minimal parameters
# Note: This assumes the Docker network is accessible from host on localhost ports.
# In reality we'd need to run this inside the creator container.
# For now, we'll just output the command that would be run.
echo "📝 Run the following command inside the creator container:"
echo "   gbn-proto upload --input test-video.mp4 --paths 3 --hops 3 --chunk-size 1048576"
echo ""
echo "⚠️  Manual validation required for now."
echo "   To automate, we need to extend proto-cli with validation flags."
echo "   However, core Docker Compose setup is complete."

# 6. Check logs for errors
echo "📊 Checking container logs for errors..."
ERRORS=$(docker-compose -f docker-compose.scale-test.yml logs 2>&1 | grep -i "error\|panic\|fatal" | head -10)
if [[ -n "$ERRORS" ]]; then
    echo "⚠️  Potential errors in logs:"
    echo "$ERRORS"
else
    echo "✅ No errors in container logs."
fi

# 7. Summary
echo ""
echo "========================================="
echo "✅ Step 5 Local Validation - Partial Complete"
echo "========================================="
echo "What's been implemented:"
echo "1. Docker Compose topology (22 nodes: 1 creator, 1 publisher, 18 hostile, 2 free)"
echo "2. Docker DNS discovery fallback (GBN_DISCOVERY_MODE=docker-dns)"
echo "3. Multi‑stage Dockerfiles (relay + publisher with ffmpeg)"
echo "4. Gossip bandwidth limiting, max tracked peers, subnet tagging"
echo ""
echo "Next steps to fully validate:"
echo "1. Run 'docker-compose -f docker-compose.scale-test.yml up -d'"
echo "2. Wait 30 seconds for network bootstrap"
echo "3. Execute upload test inside creator container"
echo "4. Manually verify gossip convergence via logs"
echo "5. Test circuit rebuild by killing a relay container"
echo ""
echo "To proceed to Step 6 (Containerization & CI/CD), update the plan document."
echo "Run: docker-compose -f docker-compose.scale-test.yml down"