#!/usr/bin/env bash
set -euo pipefail
unset HTTP_PROXY HTTPS_PROXY http_proxy https_proxy
REGION=us-east-1
STACK=gbn-proto-phase1-scale-n100
CLUSTER=${STACK}-cluster
MAX_MIN=30
STAGNANT_LIMIT=3

echo "Monitoring cluster=$CLUSTER stack=$STACK every 60s"
services=$(aws ecs list-services --cluster "$CLUSTER" --region "$REGION" --query "serviceArns[]" --output text | tr "\t" "\n" | grep -E "HostileRelayService|FreeRelayService|Creator" || true)
if [ -z "${services:-}" ]; then
  services=$(aws ecs list-services --cluster "$CLUSTER" --region "$REGION" --query "serviceArns[]" --output text | tr "\t" "\n")
fi
if [ -z "${services:-}" ]; then
  echo "TIMEOUT: no ECS services discovered in cluster"
  exit 2
fi

prev=""
stagnant=0
for ((i=1;i<=MAX_MIN;i++)); do
  ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
  svc_raw=$(aws ecs describe-services --cluster "$CLUSTER" --region "$REGION" --services $services --query "services[].[serviceName,desiredCount,runningCount,pendingCount,status]" --output text | sort)
  ec2_raw=$(aws ec2 describe-instances --region "$REGION" --filters "Name=tag:aws:cloudformation:stack-name,Values=$STACK" "Name=instance-state-name,Values=pending,running,stopping,stopped" --query "Reservations[].Instances[].State.Name" --output text || true)
  ec2_pending=$(printf "%s\n" "$ec2_raw" | tr "\t" "\n" | grep -c "^pending$" || true)
  ec2_running=$(printf "%s\n" "$ec2_raw" | tr "\t" "\n" | grep -c "^running$" || true)

  echo "[$ts] Minute $i"
  echo "$svc_raw" | sed "s/^/  ECS: /"
  echo "  EC2: running=$ec2_running pending=$ec2_pending"

  complete=1
  while read -r name desired running pending status; do
    [ -z "${name:-}" ] && continue
    if [ "$running" -lt "$desired" ] || [ "$pending" -gt 0 ]; then
      complete=0
    fi
  done <<< "$svc_raw"

  snap="$(printf "%s\nEC2:%s:%s" "$svc_raw" "$ec2_running" "$ec2_pending")"
  if [ "$snap" = "$prev" ]; then
    stagnant=$((stagnant+1))
    echo "  Progress unchanged: $stagnant/$STAGNANT_LIMIT"
  else
    stagnant=0
    echo "  Progress changed"
  fi
  prev="$snap"

  if [ "$complete" -eq 1 ]; then
    echo "SUCCESS: infra reached steady running state"
    exit 0
  fi
  if [ "$stagnant" -ge "$STAGNANT_LIMIT" ]; then
    echo "TIMEOUT: no provisioning progress for $STAGNANT_LIMIT consecutive minutes"
    exit 3
  fi

  sleep 60
done

echo "TIMEOUT: max monitoring window reached"
exit 4
