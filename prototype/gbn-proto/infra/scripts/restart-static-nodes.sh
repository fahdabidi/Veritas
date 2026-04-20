#!/usr/bin/env bash
# restart-static-nodes.sh - Enforce static EC2 node containers (SeedRelay/Publisher)
# to run with host networking and expected env wiring.
#
# Usage: ./restart-static-nodes.sh <stack-name> [region]

set -euo pipefail
export AWS_PAGER=""

STACK_NAME="${1:?Usage: $0 <stack-name> [region]}"
REGION="${2:-us-east-1}"

format_duration() {
  local total="${1:-0}"
  if [ "$total" -lt 0 ]; then
    total=0
  fi
  local minutes=$((total / 60))
  local seconds=$((total % 60))
  printf "%02dm%02ds" "$minutes" "$seconds"
}

show_countdown() {
  local elapsed="$1"
  local budget="$2"
  local remaining=$((budget - elapsed))
  if [ "$remaining" -lt 0 ]; then
    remaining=0
  fi
  printf "elapsed=%s eta~%s" "$(format_duration "$elapsed")" "$(format_duration "$remaining")"
}

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

get_ssm_ping_status() {
  local instance_id="$1"
  aws ssm describe-instance-information \
    --region "$REGION" \
    --filters "Key=InstanceIds,Values=$instance_id" \
    --query 'InstanceInformationList[0].PingStatus' \
    --output text 2>/dev/null || true
}

wait_for_ssm_online() {
  local instance_id="$1"
  local max_wait="${2:-900}"
  local interval=10
  local elapsed=0

  aws ec2 wait instance-running \
    --instance-ids "$instance_id" \
    --region "$REGION" 2>/dev/null || true
  aws ec2 wait instance-status-ok \
    --instance-ids "$instance_id" \
    --region "$REGION" 2>/dev/null || true

  while [ "$elapsed" -lt "$max_wait" ]; do
    local ping instance_state system_status instance_status
    ping="$(aws ssm describe-instance-information \
      --region "$REGION" \
      --filters "Key=InstanceIds,Values=$instance_id" \
      --query 'InstanceInformationList[0].PingStatus' \
      --output text 2>/dev/null || true)"
    if [ "$ping" = "Online" ]; then
      return 0
    fi
    instance_state="$(aws ec2 describe-instances \
      --instance-ids "$instance_id" \
      --region "$REGION" \
      --query 'Reservations[0].Instances[0].State.Name' \
      --output text 2>/dev/null || true)"
    system_status="$(aws ec2 describe-instance-status \
      --instance-ids "$instance_id" \
      --include-all-instances \
      --region "$REGION" \
      --query 'InstanceStatuses[0].SystemStatus.Status' \
      --output text 2>/dev/null || true)"
    instance_status="$(aws ec2 describe-instance-status \
      --instance-ids "$instance_id" \
      --include-all-instances \
      --region "$REGION" \
      --query 'InstanceStatuses[0].InstanceStatus.Status' \
      --output text 2>/dev/null || true)"
    echo "  [$instance_id] waiting for SSM: ping=${ping:-unknown} ec2=${instance_state:-unknown} system=${system_status:-unknown} instance=${instance_status:-unknown} $(show_countdown "$elapsed" "$max_wait")"
    sleep "$interval"
    elapsed=$((elapsed + interval))
  done

  echo "  [$instance_id] final SSM readiness failure after ${max_wait}s"
  aws ec2 describe-instances \
    --instance-ids "$instance_id" \
    --region "$REGION" \
    --query 'Reservations[0].Instances[0].[State.Name,PrivateIpAddress,PublicIpAddress,LaunchTime]' \
    --output table 2>/dev/null || true
  aws ssm describe-instance-information \
    --region "$REGION" \
    --filters "Key=InstanceIds,Values=$instance_id" \
    --output table 2>/dev/null || true
  return 1
}

reboot_instance_for_clean_ssm() {
  local instance_id="$1"
  local label="$2"
  echo "  [$label] Rebooting instance $instance_id before container restart..."
  aws ec2 reboot-instances \
    --region "$REGION" \
    --instance-ids "$instance_id" >/dev/null
  local reboot_wait=120
  local reboot_elapsed=0
  while [ "$reboot_elapsed" -lt "$reboot_wait" ]; do
    local state
    state="$(aws ec2 describe-instances \
      --region "$REGION" \
      --instance-ids "$instance_id" \
      --query 'Reservations[0].Instances[0].State.Name' \
      --output text 2>/dev/null || true)"
    echo "  [$label] reboot in progress: ec2=${state:-unknown} $(show_countdown "$reboot_elapsed" "$reboot_wait")"
    sleep 10
    reboot_elapsed=$((reboot_elapsed + 10))
    if [ "$reboot_elapsed" -ge 20 ]; then
      break
    fi
  done
  if ! wait_for_ssm_online "$instance_id" 900; then
    echo "ERROR: $label instance $instance_id did not return to SSM Online after reboot"
    exit 1
  fi
  echo "  [$label] SSM is back online; starting container restart sequence..."
  sleep 10
}

stop_start_instance_for_clean_ssm() {
  local instance_id="$1"
  local label="$2"
  echo "  [$label] SSM is disconnected; performing full stop/start recovery on $instance_id..."
  aws ec2 stop-instances \
    --region "$REGION" \
    --instance-ids "$instance_id" >/dev/null
  local stop_wait=600
  local stop_elapsed=0
  while [ "$stop_elapsed" -lt "$stop_wait" ]; do
    local state
    state="$(aws ec2 describe-instances \
      --region "$REGION" \
      --instance-ids "$instance_id" \
      --query 'Reservations[0].Instances[0].State.Name' \
      --output text 2>/dev/null || true)"
    echo "  [$label] stopping instance: ec2=${state:-unknown} $(show_countdown "$stop_elapsed" "$stop_wait")"
    if [ "$state" = "stopped" ]; then
      break
    fi
    sleep 10
    stop_elapsed=$((stop_elapsed + 10))
  done
  aws ec2 wait instance-stopped \
    --region "$REGION" \
    --instance-ids "$instance_id"

  aws ec2 start-instances \
    --region "$REGION" \
    --instance-ids "$instance_id" >/dev/null
  echo "  [$label] instance start issued; waiting for EC2/SSM recovery..."
  if ! wait_for_ssm_online "$instance_id" 900; then
    echo "ERROR: $label instance $instance_id did not return to SSM Online after stop/start"
    exit 1
  fi
  echo "  [$label] stop/start recovery succeeded; starting container restart sequence..."
  sleep 10
}

recover_instance_for_clean_ssm() {
  local instance_id="$1"
  local label="$2"
  local ping
  ping="$(get_ssm_ping_status "$instance_id")"
  echo "  [$label] preflight SSM ping status: ${ping:-unknown}"
  if [ "$ping" = "Online" ]; then
    reboot_instance_for_clean_ssm "$instance_id" "$label"
  else
    stop_start_instance_for_clean_ssm "$instance_id" "$label"
  fi
}

run_ssm_commands() {
  local instance_id="$1"
  local container_name="$2"
  local image_uri="$3"
  local docker_envs="${4:-}"
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
      "echo '[remote] step=cleanup-containers'",
      "docker rm -f \$(docker ps -aq) 2>/dev/null || true",
      "echo '[remote] step=cleanup-images'",
      "docker image rm -f \$(docker images -aq) 2>/dev/null || true",
      "echo '[remote] step=system-prune'",
      "docker system prune -af --volumes || true",
      "echo '[remote] step=ecr-login'",
      "aws ecr get-login-password --region $REGION | docker login --username AWS --password-stdin $registry",
      "echo '[remote] step=docker-pull image=$image_uri'",
      "docker pull $image_uri",
      "echo '[remote] step=remove-existing name=$container_name'",
      "docker rm -f $container_name || true",
      "echo '[remote] step=docker-run name=$container_name'",
      "docker run -d --restart always --name $container_name --network host $docker_envs $image_uri",
      "echo '[remote] step=verify-container name=$container_name'",
      "docker ps --format '{{.Names}} {{.Image}}' | grep $container_name"
    ]
  }
}
JSON

  local max_attempts=3
  local status=""
  local stdout=""
  local stderr=""

  local attempt=1
  while [ "$attempt" -le "$max_attempts" ]; do
    local cmd_id
    cmd_id="$(aws ssm send-command \
      --region "$REGION" \
      --cli-input-json "$(<"$cmd_json_file")" \
      --query 'Command.CommandId' \
      --output text)"

    local max_wait=900
    local delivery_grace=90
    local interval=5
    local elapsed=0
    local last_progress=""

    echo "  [$container_name] SSM attempt $attempt/$max_attempts command_id=$cmd_id"

    while [ "$elapsed" -lt "$max_wait" ]; do
      status="$(aws ssm get-command-invocation \
        --region "$REGION" \
        --command-id "$cmd_id" \
        --instance-id "$instance_id" \
        --query 'Status' \
        --output text 2>/dev/null || echo Pending)"
      local status_details
      status_details="$(aws ssm get-command-invocation \
        --region "$REGION" \
        --command-id "$cmd_id" \
        --instance-id "$instance_id" \
        --query 'StatusDetails' \
        --output text 2>/dev/null || echo Pending)"

      case "$status" in
        Success|Failed|Cancelled|TimedOut|Undeliverable|Terminated|Delivery\ Timed\ Out|Execution\ Timed\ Out)
          break
          ;;
      esac

      local progress_preview
      progress_preview="$(aws ssm get-command-invocation \
        --region "$REGION" \
        --command-id "$cmd_id" \
        --instance-id "$instance_id" \
        --query 'StandardOutputContent' \
        --output text 2>/dev/null | tail -n 3 | tr '\n' '|' || true)"
      if [ -n "$progress_preview" ] && [ "$progress_preview" != "$last_progress" ]; then
        echo "  [$container_name] progress: $progress_preview"
        last_progress="$progress_preview"
      else
        echo "  [$container_name] status=$status details=$status_details $(show_countdown "$elapsed" "$max_wait")"
      fi

      if [ "$elapsed" -ge "$delivery_grace" ] && { [ "$status" = "Pending" ] || [ "$status_details" = "Delayed" ]; }; then
        echo "  [$container_name] SSM delivery stuck in status=$status details=$status_details after ${elapsed}s; canceling and retrying..."
        aws ssm cancel-command \
          --region "$REGION" \
          --command-id "$cmd_id" >/dev/null 2>&1 || true
        status="RetryableDelayed"
        break
      fi

      sleep "$interval"
      elapsed=$((elapsed + interval))
    done

    stdout="$(aws ssm get-command-invocation \
      --region "$REGION" \
      --command-id "$cmd_id" \
      --instance-id "$instance_id" \
      --query 'StandardOutputContent' \
      --output text 2>/dev/null || true)"
    stderr="$(aws ssm get-command-invocation \
      --region "$REGION" \
      --command-id "$cmd_id" \
      --instance-id "$instance_id" \
      --query 'StandardErrorContent' \
      --output text 2>/dev/null || true)"
    if [ "$status" != "RetryableDelayed" ]; then
      status="$(aws ssm get-command-invocation \
        --region "$REGION" \
        --command-id "$cmd_id" \
        --instance-id "$instance_id" \
        --query 'Status' \
        --output text 2>/dev/null || echo "$status")"
    fi

    echo "  [$container_name] SSM status: $status"
    printf "%s\n" "$stdout"

    if [ "$status" = "Success" ]; then
      rm -f "$cmd_json_file"
      return 0
    fi

    if [ "$status" != "RetryableDelayed" ] || [ "$attempt" -ge "$max_attempts" ]; then
      echo "ERROR: Failed to restart $container_name on $instance_id"
      printf "%s\n" "$stderr"
      rm -f "$cmd_json_file"
      exit 1
    fi

    attempt=$((attempt + 1))
    sleep 5
  done

  rm -f "$cmd_json_file"
}

restart_static_container() {
  local instance_id="$1"
  local container_name="$2"
  local image_uri="$3"
  local docker_envs="${4:-}"

  recover_instance_for_clean_ssm "$instance_id" "$container_name"
  run_ssm_commands "$instance_id" "$container_name" "$image_uri" "$docker_envs"
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

SEED_RELAY_PRIVATE_IP="$(aws ec2 describe-instances \
  --instance-ids "$SEED_RELAY_INSTANCE_ID" \
  --region "$REGION" \
  --query 'Reservations[0].Instances[0].PrivateIpAddress' \
  --output text 2>/dev/null || true)"
if [ -z "$SEED_RELAY_PRIVATE_IP" ] || [ "$SEED_RELAY_PRIVATE_IP" = "None" ]; then
  echo "ERROR: Could not resolve SeedRelay private IP for $SEED_RELAY_INSTANCE_ID"
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
echo "  [gbn-seed-relay] current ping status: $(get_ssm_ping_status "$SEED_RELAY_INSTANCE_ID")"
echo "  [gbn-publisher] current ping status: $(get_ssm_ping_status "$PUBLISHER_INSTANCE_ID")"
echo "  Static nodes will auto-recover with stop/start if SSM is disconnected."

echo "Restarting SeedRelay with host networking..."
SEED_ENVS="-e GBN_ROLE=relay -e GBN_P2P_PORT=4001 -e GBN_ONION_PORT=9001 -e GBN_CONTROL_PORT=5050 -e GBN_INSTANCE_IPV4=10.0.1.10 -e GBN_SUBNET_TAG=HostileSubnet -e GBN_NOISE_PRIVKEY_HEX=$SEED_RELAY_NOISE_PRIVKEY -e RUST_LOG=info"
echo "Restarting Publisher with host networking..."
PUBLISHER_ENVS="-e GBN_ROLE=publisher -e GBN_P2P_PORT=4001 -e GBN_MPUB_PORT=7001 -e GBN_CONTROL_PORT=5050 -e GBN_SEED_IPS=${SEED_RELAY_PRIVATE_IP}:4001 -e GBN_INSTANCE_IPV4=10.0.3.10 -e GBN_SUBNET_TAG=PublisherNode -e GBN_PUBLISHER_KEY_HEX=$PUBLISHER_KEY_HEX -e RUST_LOG=info"
restart_static_container "$SEED_RELAY_INSTANCE_ID" "gbn-seed-relay" "$CONTAINER_IMAGE_RELAY" "$SEED_ENVS" &
seed_pid=$!
restart_static_container "$PUBLISHER_INSTANCE_ID" "gbn-publisher" "$CONTAINER_IMAGE_PUBLISHER" "$PUBLISHER_ENVS" &
publisher_pid=$!

wait "$seed_pid"
wait "$publisher_pid"

echo "✅ Static EC2 nodes are running with --network host."
