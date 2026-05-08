#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REGION="${GBN_BRIDGE_AWS_REGION:-${AWS_REGION:-us-east-1}}"
TAG="${GBN_BRIDGE_IMAGE_TAG:-latest}"
AUTHORITY_REPO="${GBN_BRIDGE_AUTHORITY_REPO:-gbn-conduit-full-authority}"
RECEIVER_REPO="${GBN_BRIDGE_RECEIVER_REPO:-gbn-conduit-full-receiver}"
BRIDGE_REPO="${GBN_BRIDGE_EXIT_BRIDGE_REPO:-gbn-conduit-full-bridge}"
CREATOR_REPO="${GBN_BRIDGE_CREATOR_REPO:-gbn-conduit-full-creator}"

usage() {
  cat <<USAGE
Usage: $0 [--region REGION] [--tag TAG]

Build and push the Conduit full-stack authority, receiver, bridge, and creator images to ECR.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --region) REGION="$2"; shift 2 ;;
    --tag) TAG="$2"; shift 2 ;;
    --help|-h) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "required command not found: $1" >&2
    exit 127
  }
}

ensure_repo() {
  local repo="$1"
  if ! aws ecr describe-repositories --region "$REGION" --repository-names "$repo" >/dev/null 2>&1; then
    aws ecr create-repository \
      --region "$REGION" \
      --repository-name "$repo" \
      --image-scanning-configuration scanOnPush=true >/dev/null
  fi
}

require_cmd aws
require_cmd docker

ACCOUNT_ID="$(aws sts get-caller-identity --query Account --output text)"
REGISTRY="${ACCOUNT_ID}.dkr.ecr.${REGION}.amazonaws.com"
AUTHORITY_URI="${REGISTRY}/${AUTHORITY_REPO}:${TAG}"
RECEIVER_URI="${REGISTRY}/${RECEIVER_REPO}:${TAG}"
BRIDGE_URI="${REGISTRY}/${BRIDGE_REPO}:${TAG}"
CREATOR_URI="${REGISTRY}/${CREATOR_REPO}:${TAG}"

ensure_repo "$AUTHORITY_REPO"
ensure_repo "$RECEIVER_REPO"
ensure_repo "$BRIDGE_REPO"
ensure_repo "$CREATOR_REPO"

aws ecr get-login-password --region "$REGION" \
  | docker login --username AWS --password-stdin "$REGISTRY"

docker build -f "$ROOT_DIR/Dockerfile.publisher-authority" -t "$AUTHORITY_URI" "$ROOT_DIR"
docker build -f "$ROOT_DIR/Dockerfile.publisher-receiver" -t "$RECEIVER_URI" "$ROOT_DIR"
docker build -f "$ROOT_DIR/Dockerfile.bridge" -t "$BRIDGE_URI" "$ROOT_DIR"
docker build -f "$ROOT_DIR/Dockerfile.creator-runner" -t "$CREATOR_URI" "$ROOT_DIR"

docker push "$AUTHORITY_URI"
docker push "$RECEIVER_URI"
docker push "$BRIDGE_URI"
docker push "$CREATOR_URI"

cat <<IMAGES
AuthorityImageUri=$AUTHORITY_URI
ReceiverImageUri=$RECEIVER_URI
BridgeImageUri=$BRIDGE_URI
CreatorImageUri=$CREATOR_URI
IMAGES
