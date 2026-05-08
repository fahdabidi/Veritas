#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEMPLATE="$ROOT_DIR/infra/cloudformation/conduit-full-stack.yaml"
STACK_NAME="${GBN_BRIDGE_STACK_NAME:-gbn-conduit-full-dev}"
REGION="${GBN_BRIDGE_AWS_REGION:-${AWS_REGION:-us-east-1}}"
ENVIRONMENT_NAME="${GBN_BRIDGE_ENVIRONMENT:-dev}"
ASSIGN_PUBLIC_IP="${GBN_BRIDGE_ASSIGN_PUBLIC_IP:-ENABLED}"
DESIRED_BRIDGE_COUNT="${GBN_BRIDGE_DESIRED_BRIDGE_COUNT:-10}"
AUTHORITY_PORT="${GBN_BRIDGE_AUTHORITY_PORT:-8080}"
RECEIVER_PORT="${GBN_BRIDGE_RECEIVER_PORT:-8081}"
UDP_PUNCH_PORT="${GBN_BRIDGE_PUNCH_PORT:-443}"
DATABASE_NAME="${GBN_BRIDGE_DATABASE_NAME:-veritas_conduit}"
DATABASE_USER="${GBN_BRIDGE_DATABASE_USER:-veritas}"
DATABASE_INSTANCE_CLASS="${GBN_BRIDGE_DATABASE_INSTANCE_CLASS:-db.t3.micro}"
DATABASE_ALLOCATED_STORAGE="${GBN_BRIDGE_DATABASE_ALLOCATED_STORAGE:-20}"
POSTGRES_TLS_ACCEPT_INVALID_CERTS="${GBN_BRIDGE_POSTGRES_TLS_ACCEPT_INVALID_CERTS:-false}"
AUTHORITY_INGRESS_CIDR="${GBN_BRIDGE_AUTHORITY_INGRESS_CIDR:-0.0.0.0/0}"

usage() {
  cat <<USAGE
Usage: $0 --vpc-id VPC --service-subnet-ids SUBNET_A,SUBNET_B --database-subnet-ids SUBNET_C,SUBNET_D \
  --authority-image URI --receiver-image URI --bridge-image URI --creator-image URI \
  --publisher-signing-key-secret-arn ARN --bridge-signing-seed-secret-arn ARN --publisher-public-key-hex HEX [options]

Options:
  --stack-name NAME
  --region REGION
  --environment NAME
  --assign-public-ip ENABLED|DISABLED
  --desired-bridge-count COUNT
  --authority-port PORT
  --receiver-port PORT
  --udp-punch-port PORT
  --database-name NAME
  --database-user NAME
  --database-instance-class CLASS
  --database-allocated-storage GB
  --postgres-tls-accept-invalid-certs true|false
  --authority-ingress-cidr CIDR
USAGE
}

VPC_ID=""
SERVICE_SUBNET_IDS=""
DATABASE_SUBNET_IDS=""
AUTHORITY_IMAGE_URI=""
RECEIVER_IMAGE_URI=""
BRIDGE_IMAGE_URI=""
CREATOR_IMAGE_URI=""
PUBLISHER_SIGNING_KEY_SECRET_ARN=""
BRIDGE_SIGNING_SEED_SECRET_ARN=""
PUBLISHER_PUBLIC_KEY_HEX=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --stack-name) STACK_NAME="$2"; shift 2 ;;
    --region) REGION="$2"; shift 2 ;;
    --environment) ENVIRONMENT_NAME="$2"; shift 2 ;;
    --vpc-id) VPC_ID="$2"; shift 2 ;;
    --service-subnet-ids) SERVICE_SUBNET_IDS="$2"; shift 2 ;;
    --database-subnet-ids) DATABASE_SUBNET_IDS="$2"; shift 2 ;;
    --authority-image) AUTHORITY_IMAGE_URI="$2"; shift 2 ;;
    --receiver-image) RECEIVER_IMAGE_URI="$2"; shift 2 ;;
    --bridge-image) BRIDGE_IMAGE_URI="$2"; shift 2 ;;
    --creator-image) CREATOR_IMAGE_URI="$2"; shift 2 ;;
    --publisher-signing-key-secret-arn) PUBLISHER_SIGNING_KEY_SECRET_ARN="$2"; shift 2 ;;
    --bridge-signing-seed-secret-arn) BRIDGE_SIGNING_SEED_SECRET_ARN="$2"; shift 2 ;;
    --publisher-public-key-hex) PUBLISHER_PUBLIC_KEY_HEX="$2"; shift 2 ;;
    --assign-public-ip) ASSIGN_PUBLIC_IP="$2"; shift 2 ;;
    --desired-bridge-count) DESIRED_BRIDGE_COUNT="$2"; shift 2 ;;
    --authority-port) AUTHORITY_PORT="$2"; shift 2 ;;
    --receiver-port) RECEIVER_PORT="$2"; shift 2 ;;
    --udp-punch-port) UDP_PUNCH_PORT="$2"; shift 2 ;;
    --database-name) DATABASE_NAME="$2"; shift 2 ;;
    --database-user) DATABASE_USER="$2"; shift 2 ;;
    --database-instance-class) DATABASE_INSTANCE_CLASS="$2"; shift 2 ;;
    --database-allocated-storage) DATABASE_ALLOCATED_STORAGE="$2"; shift 2 ;;
    --postgres-tls-accept-invalid-certs) POSTGRES_TLS_ACCEPT_INVALID_CERTS="$2"; shift 2 ;;
    --authority-ingress-cidr) AUTHORITY_INGRESS_CIDR="$2"; shift 2 ;;
    --help|-h) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [[ "$STACK_NAME" != gbn-conduit-full-* ]]; then
  echo "stack name must start with gbn-conduit-full-: $STACK_NAME" >&2
  exit 2
fi

if [[ -z "$VPC_ID" || -z "$SERVICE_SUBNET_IDS" || -z "$DATABASE_SUBNET_IDS" || -z "$AUTHORITY_IMAGE_URI" || -z "$RECEIVER_IMAGE_URI" || -z "$BRIDGE_IMAGE_URI" || -z "$CREATOR_IMAGE_URI" || -z "$PUBLISHER_SIGNING_KEY_SECRET_ARN" || -z "$BRIDGE_SIGNING_SEED_SECRET_ARN" || -z "$PUBLISHER_PUBLIC_KEY_HEX" ]]; then
  usage >&2
  exit 2
fi

command -v aws >/dev/null 2>&1 || {
  echo "required command not found: aws" >&2
  exit 127
}

aws cloudformation deploy \
  --region "$REGION" \
  --stack-name "$STACK_NAME" \
  --template-file "$TEMPLATE" \
  --capabilities CAPABILITY_NAMED_IAM \
  --parameter-overrides \
    EnvironmentName="$ENVIRONMENT_NAME" \
    VpcId="$VPC_ID" \
    ServiceSubnetIds="$SERVICE_SUBNET_IDS" \
    DatabaseSubnetIds="$DATABASE_SUBNET_IDS" \
    AssignPublicIp="$ASSIGN_PUBLIC_IP" \
    AuthorityImageUri="$AUTHORITY_IMAGE_URI" \
    ReceiverImageUri="$RECEIVER_IMAGE_URI" \
    BridgeImageUri="$BRIDGE_IMAGE_URI" \
    CreatorImageUri="$CREATOR_IMAGE_URI" \
    DesiredBridgeCount="$DESIRED_BRIDGE_COUNT" \
    AuthorityPort="$AUTHORITY_PORT" \
    ReceiverPort="$RECEIVER_PORT" \
    UdpPunchPort="$UDP_PUNCH_PORT" \
    DatabaseName="$DATABASE_NAME" \
    DatabaseUsername="$DATABASE_USER" \
    DatabaseInstanceClass="$DATABASE_INSTANCE_CLASS" \
    DatabaseAllocatedStorage="$DATABASE_ALLOCATED_STORAGE" \
    PostgresTlsAcceptInvalidCerts="$POSTGRES_TLS_ACCEPT_INVALID_CERTS" \
    PublisherSigningKeySecretArn="$PUBLISHER_SIGNING_KEY_SECRET_ARN" \
    BridgeSigningSeedSecretArn="$BRIDGE_SIGNING_SEED_SECRET_ARN" \
    PublisherPublicKeyHex="$PUBLISHER_PUBLIC_KEY_HEX" \
    AuthorityIngressCidr="$AUTHORITY_INGRESS_CIDR"

"$ROOT_DIR/infra/scripts/smoke-conduit-full.sh" --stack-name "$STACK_NAME" --region "$REGION"
