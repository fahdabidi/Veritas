#!/usr/bin/env bash
# build-and-push.sh — Build Docker images and push to Amazon ECR for GBN Phase 1 Scale Test.
#
# Usage: ./build-and-push.sh <stack-name> [region]

set -euo pipefail
export AWS_PAGER=""

if ! command -v aws >/dev/null 2>&1; then
  if command -v aws.exe >/dev/null 2>&1; then
    AWS_IS_EXE=1
    aws() { aws.exe "$@"; }
  else
    echo "ERROR: aws CLI not found in PATH (tried aws and aws.exe)."
    exit 1
  fi
fi

AWS_IS_EXE="${AWS_IS_EXE:-0}"

if ! command -v docker >/dev/null 2>&1; then
  if command -v docker.exe >/dev/null 2>&1; then
    docker() { docker.exe "$@"; }
  else
    echo "ERROR: docker not found in PATH (tried docker and docker.exe)."
    exit 1
  fi
fi

STACK_NAME="${1:?Usage: $0 <stack-name> [region]}"
REGION="${2:-us-east-1}"

cf_output() {
  local key="$1"
  aws cloudformation describe-stacks --stack-name "$STACK_NAME" --region "$REGION" --output json | \
    python -c "import json,sys; d=json.load(sys.stdin); o=d['Stacks'][0].get('Outputs',[]); print(next((x['OutputValue'] for x in o if x.get('OutputKey')=='$key'), ''))"
}

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROTO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "============================================"
echo "  GBN Phase 1 — Build & Push to ECR"
echo "  Stack:  $STACK_NAME"
echo "  Region: $REGION"
echo "============================================"

echo "[1/4] Resolving stack outputs..."
ECR_URI="$(cf_output ECRUri)"

if [ -z "$ECR_URI" ]; then
  echo "ERROR: Missing CloudFormation output 'ECRUri'."
  exit 1
fi

echo "  ECR Repository: $ECR_URI"

echo "[2/4] Determining git SHA..."
cd "$PROTO_ROOT"
if ! git rev-parse --short HEAD >/dev/null 2>&1; then
  echo "WARNING: Not a git repository, using 'local' as SHA."
  GIT_SHA="local"
else
  GIT_SHA="$(git rev-parse --short HEAD)"
fi
echo "  Git SHA: $GIT_SHA"

echo "[3/4] Building Docker images..."
# Build relay image (no ffmpeg)
docker build -t gbn-relay -f "$PROTO_ROOT/Dockerfile.relay" "$PROTO_ROOT"
docker tag gbn-relay "${ECR_URI}/gbn-relay:${GIT_SHA}"
docker tag gbn-relay "${ECR_URI}/gbn-relay:latest"

# Build publisher image (includes ffmpeg)
docker build -t gbn-publisher -f "$PROTO_ROOT/Dockerfile.publisher" "$PROTO_ROOT"
docker tag gbn-publisher "${ECR_URI}/gbn-publisher:${GIT_SHA}"
docker tag gbn-publisher "${ECR_URI}/gbn-publisher:latest"

echo "[4/4] Logging into ECR and pushing images..."
aws ecr get-login-password --region "$REGION" | docker login --username AWS --password-stdin "$ECR_URI"

for image in gbn-relay gbn-publisher; do
  echo "  Pushing ${image}:${GIT_SHA}"
  docker push "${ECR_URI}/${image}:${GIT_SHA}"
  echo "  Pushing ${image}:latest"
  docker push "${ECR_URI}/${image}:latest"
done

echo ""
echo "✅ All images pushed successfully."
echo "   ECR Repository: $ECR_URI"
echo "   Relay image:    ${ECR_URI}/gbn-relay:${GIT_SHA}"
echo "   Publisher image: ${ECR_URI}/gbn-publisher:${GIT_SHA}"
echo ""
echo "To deploy the latest images, update your ECS Task Definitions to use:"
echo "  image: ${ECR_URI}/gbn-relay:latest   (or :${GIT_SHA} for pinning)"
echo "  image: ${ECR_URI}/gbn-publisher:latest"