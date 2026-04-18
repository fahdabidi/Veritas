#!/usr/bin/env bash
# restart-static-nodes.sh - Enforce static EC2 node containers (SeedRelay/Publisher)
# to run with host networking and expected env wiring.
#
# Usage: ./restart-static-nodes.sh <stack-name> [region]

set -euo pipefail
export AWS_PAGER=""

STACK_NAME="${1:?Usage: $0 <stack-name> [region]}"
REGION="${2:-us-east-1}"

cf_resource_id() {
  local logical_id="$1"
  aws cloudformation describe-stack-resources \
    --stack-name "$STACK_NAME" \
    --region "$REGION" \
    --logical-resource-id "$logical_id" \
    --query 'StackResources[0].PhysicalResourceId' \
    --output text
}

cf_parameter() {
  local key="$1"
  aws cloudformation describe-stacks \
    --stack-name "$STACK_NAME" \
    --region "$REGION" \
    --query "Stacks[0].Parameters[?ParameterKey==\`$key\`].ParameterValue | [0]" \
    --output text
}

wait_for_ssm_online() {
  local instance_id="$1"
  local max_wait="${2:-300}"
  local interval=5
  local elapsed=0
  while [ "$elapsed" -lt "$max_wait" ]; do
    local ping
    ping="$(aws ssm describe-instance-information \
      --region "$REGION" \
      --filters "Key=InstanceIds,Values=$instance_id" \
      --query 'InstanceInformationList[0].PingStatus' \
      --output text 2>/dev/null || true)"
    if [ "$ping" = "Online" ]; then
      return 0
    fi
    sleep "$interval"
    elapsed=$((elapsed + interval))
  done
  return 1
}

run_ssm_commands() {
  local instance_id="$1"
  local container_name="$2"
  local image_uri="$3"
  local registry
  registry="${image_uri%%/*}"
  local cmd_json_file
  cmd_json_file="$(mktemp)"
  cat > "$cmd_json_file" <<JSON
{
  "DocumentName": "AWS-RunShellScript",
  "InstanceIds": ["$instance_id"],
  "Parameters": {
    "commands": [
      "set -e",
      "docker rm -f \$(docker ps -aq) 2>/dev/null || true",
      "docker image rm -f \$(docker images -aq) 2>/dev/null || true",
      "docker system prune -af --volumes || true",
      "aws ecr get-login-password --region $REGION | docker login --username AWS --password-stdin $registry",
      "docker pull $image_uri",
      "docker rm -f $container_name || true",
      "docker run -d --restart always --name $container_name --network host $4 $image_uri",
      "docker ps --format '{{.Names}} {{.Image}}' | grep $container_name"
    ]
  }
}
JSON

  local cmd_id
  cmd_id="$(aws ssm send-command \
    --region "$REGION" \
    --cli-input-json "$(<"$cmd_json_file")" \
    --query 'Command.CommandId' \
    --output text)"

  if ! aws ssm wait command-executed --region "$REGION" --command-id "$cmd_id" --instance-id "$instance_id"; then
    echo "  [$container_name] waiter reported non-success, collecting invocation details..."
  fi

  local status
  local stdout
  local stderr
  status="$(aws ssm get-command-invocation \
    --region "$REGION" \
    --command-id "$cmd_id" \
    --instance-id "$instance_id" \
    --query 'Status' \
    --output text)"
  stdout="$(aws ssm get-command-invocation \
    --region "$REGION" \
    --command-id "$cmd_id" \
    --instance-id "$instance_id" \
    --query 'StandardOutputContent' \
    --output text)"
  stderr="$(aws ssm get-command-invocation \
    --region "$REGION" \
    --command-id "$cmd_id" \
    --instance-id "$instance_id" \
    --query 'StandardErrorContent' \
    --output text)"

  echo "  [$container_name] SSM status: $status"
  printf "%s\n" "$stdout"

  if [ "$status" != "Success" ]; then
    echo "ERROR: Failed to restart $container_name on $instance_id"
    printf "%s\n" "$stderr"
    rm -f "$cmd_json_file"
    exit 1
  fi

  rm -f "$cmd_json_file"
}

SEED_RELAY_INSTANCE_ID="$(cf_resource_id SeedRelayInstance)"
PUBLISHER_INSTANCE_ID="$(cf_resource_id PublisherInstance)"

if [ -z "$SEED_RELAY_INSTANCE_ID" ] || [ "$SEED_RELAY_INSTANCE_ID" = "None" ]; then
  echo "ERROR: SeedRelayInstance not found in stack resources."
  exit 1
fi
if [ -z "$PUBLISHER_INSTANCE_ID" ] || [ "$PUBLISHER_INSTANCE_ID" = "None" ]; then
  echo "ERROR: PublisherInstance not found in stack resources."
  exit 1
fi

SEED_RELAY_NOISE_PRIVKEY="$(cf_parameter SeedRelayNoisePrivKey)"
PUBLISHER_KEY_HEX="$(cf_parameter PublisherPrivKeyHex)"
CONTAINER_IMAGE_RELAY="$(cf_parameter ContainerImageRelay)"
CONTAINER_IMAGE_PUBLISHER="$(cf_parameter ContainerImagePublisher)"

if [ -z "$SEED_RELAY_NOISE_PRIVKEY" ] || [ "$SEED_RELAY_NOISE_PRIVKEY" = "None" ]; then
  echo "ERROR: SeedRelayNoisePrivKey parameter missing."
  exit 1
fi
if [ -z "$PUBLISHER_KEY_HEX" ] || [ "$PUBLISHER_KEY_HEX" = "None" ]; then
  echo "ERROR: PublisherPrivKeyHex parameter missing."
  exit 1
fi

if [ -z "$CONTAINER_IMAGE_RELAY" ] || [ "$CONTAINER_IMAGE_RELAY" = "None" ]; then
  ACCOUNT_ID="$(aws sts get-caller-identity --query 'Account' --output text --region "$REGION")"
  CONTAINER_IMAGE_RELAY="${ACCOUNT_ID}.dkr.ecr.${REGION}.amazonaws.com/${STACK_NAME}-gbn-relay:latest"
fi
if [ -z "$CONTAINER_IMAGE_PUBLISHER" ] || [ "$CONTAINER_IMAGE_PUBLISHER" = "None" ]; then
  ACCOUNT_ID="$(aws sts get-caller-identity --query 'Account' --output text --region "$REGION")"
  CONTAINER_IMAGE_PUBLISHER="${ACCOUNT_ID}.dkr.ecr.${REGION}.amazonaws.com/${STACK_NAME}-gbn-publisher:latest"
fi

echo "Ensuring SSM connectivity..."
if ! wait_for_ssm_online "$SEED_RELAY_INSTANCE_ID"; then
  echo "ERROR: SeedRelay instance did not become SSM-online: $SEED_RELAY_INSTANCE_ID"
  exit 1
fi
if ! wait_for_ssm_online "$PUBLISHER_INSTANCE_ID"; then
  echo "ERROR: Publisher instance did not become SSM-online: $PUBLISHER_INSTANCE_ID"
  exit 1
fi

echo "Restarting SeedRelay with host networking..."
SEED_ENVS="-e GBN_ROLE=relay -e GBN_P2P_PORT=4001 -e GBN_ONION_PORT=9001 -e GBN_CONTROL_PORT=5050 -e GBN_INSTANCE_IPV4=10.0.1.10 -e GBN_SUBNET_TAG=HostileSubnet -e GBN_NOISE_PRIVKEY_HEX=$SEED_RELAY_NOISE_PRIVKEY -e RUST_LOG=info"
run_ssm_commands "$SEED_RELAY_INSTANCE_ID" "gbn-seed-relay" "$CONTAINER_IMAGE_RELAY" "$SEED_ENVS"

echo "Restarting Publisher with host networking..."
PUBLISHER_ENVS="-e GBN_ROLE=publisher -e GBN_P2P_PORT=4001 -e GBN_MPUB_PORT=7001 -e GBN_CONTROL_PORT=5050 -e GBN_SEED_IPS=10.0.1.10:4001 -e GBN_INSTANCE_IPV4=10.0.3.10 -e GBN_SUBNET_TAG=FreeSubnet -e GBN_PUBLISHER_KEY_HEX=$PUBLISHER_KEY_HEX -e RUST_LOG=info"
run_ssm_commands "$PUBLISHER_INSTANCE_ID" "gbn-publisher" "$CONTAINER_IMAGE_PUBLISHER" "$PUBLISHER_ENVS"

echo "✅ Static EC2 nodes are running with --network host."
