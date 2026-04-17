#!/usr/bin/env bash
set -euo pipefail
AWS_REGION=us-east-1
CLUSTER=gbn-proto-phase1-scale-n100-cluster
STACK=gbn-proto-phase1-scale-n100
TARGET_IPS=(10.0.0.117 10.0.0.177 10.0.3.96)

HOSTILE_SVC=$(aws ecs list-services --cluster "$CLUSTER" --region "$AWS_REGION" --query "serviceArns[?contains(@,\`HostileRelayService\`)]|[0]" --output text)
FREE_SVC=$(aws ecs list-services --cluster "$CLUSTER" --region "$AWS_REGION" --query "serviceArns[?contains(@,\`FreeRelayService\`)]|[0]" --output text)
TASKS="$(aws ecs list-tasks --cluster "$CLUSTER" --service-name "$HOSTILE_SVC" --desired-status RUNNING --region "$AWS_REGION" --query taskArns --output text) $(aws ecs list-tasks --cluster "$CLUSTER" --service-name "$FREE_SVC" --desired-status RUNNING --region "$AWS_REGION" --query taskArns --output text)"
TASKS=$(echo "$TASKS" | xargs)
DESC=$(aws ecs describe-tasks --cluster "$CLUSTER" --tasks $TASKS --region "$AWS_REGION" --query "tasks[].{task:taskArn,ip:attachments[0].details[?name==\`privateIPv4Address\`]|[0].value}" --output text)

echo "=== Relay task mapping (ip -> taskArn) ==="
echo "$DESC"

SEED=$(aws cloudformation describe-stack-resources --stack-name "$STACK" --logical-resource-id SeedRelayInstance --region "$AWS_REGION" --query "StackResources[0].PhysicalResourceId" --output text)
CID=$(aws ssm send-command --instance-ids "$SEED" --document-name AWS-RunShellScript --parameters "commands=sudo docker exec gbn-seed-relay python3 -c 'import socket; s=socket.create_connection((\"127.0.0.1\",5050),2); s.sendall(b\"{\\\"cmd\\\":\\\"DumpMetadata\\\"}\\n\"); print(s.recv(65535).decode()); s.close()'" --region "$AWS_REGION" --query "Command.CommandId" --output text)
sleep 3
echo "=== DumpMetadata from SeedRelay EC2 container ==="
aws ssm list-command-invocations --command-id "$CID" --details --region "$AWS_REGION" --query "CommandInvocations[0].CommandPlugins[0].Output" --output text

for ip in "${TARGET_IPS[@]}"; do
  echo "=== DumpMetadata from relay ip $ip ==="
  task=$(echo "$DESC" | awk -v target="$ip" '$1==target{print $2}')
  if [[ -z "${task:-}" ]]; then
    echo "NOT_FOUND: no running relay task currently has ip $ip"
    continue
  fi
  aws ecs execute-command --cluster "$CLUSTER" --task "$task" --container relay --interactive --region "$AWS_REGION" --command "python3 -c \"import socket; s=socket.create_connection(('127.0.0.1',5050),2); s.sendall(b'{\\\"cmd\\\":\\\"DumpMetadata\\\"}\\n'); print(s.recv(65535).decode()); s.close()\""
done
