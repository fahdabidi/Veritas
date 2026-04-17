set -euo pipefail
export AWS_REGION=us-east-1
export STACK_NAME=gbn-proto-phase1-scale-n100

CLUSTER_ARN=$(aws cloudformation describe-stack-resources --stack-name "$STACK_NAME" --logical-resource-id ECSCluster --query 'StackResources[0].PhysicalResourceId' --output text --region "$AWS_REGION")
CREATOR_SVC_ARN=$(aws cloudformation describe-stack-resources --stack-name "$STACK_NAME" --logical-resource-id CreatorService --query 'StackResources[0].PhysicalResourceId' --output text --region "$AWS_REGION")
RELAY_SVC_ARN=$(aws cloudformation describe-stack-resources --stack-name "$STACK_NAME" --logical-resource-id FreeRelayService --query 'StackResources[0].PhysicalResourceId' --output text --region "$AWS_REGION")

echo "CLUSTER_ARN=$CLUSTER_ARN"
echo "CREATOR_SVC=$CREATOR_SVC_ARN"
echo "RELAY_SVC=$RELAY_SVC_ARN"

CREATOR_TASK=$(aws ecs list-tasks --cluster "$CLUSTER_ARN" --service "$CREATOR_SVC_ARN" --desired-status RUNNING --query 'taskArns[0]' --output text --region "$AWS_REGION")
RELAY_TASK=$(aws ecs list-tasks --cluster "$CLUSTER_ARN" --service "$RELAY_SVC_ARN" --desired-status RUNNING --query 'taskArns[0]' --output text --region "$AWS_REGION")

echo "CREATOR_TASK=$CREATOR_TASK"
echo "RELAY_TASK=$RELAY_TASK"

if [ -z "$CREATOR_TASK" ] || [ "$CREATOR_TASK" = "None" ] || [ -z "$RELAY_TASK" ] || [ "$RELAY_TASK" = "None" ]; then
  echo "Creator and/or relay task is not RUNNING"
  exit 1
fi

RELAY_SERVICE_ID=$(aws cloudformation describe-stacks --stack-name "$STACK_NAME" --region "$AWS_REGION" --query "Stacks[0].Outputs[?OutputKey=='RelayDiscoveryServiceId'].OutputValue | [0]" --output text)
RELAY_IP=$(aws servicediscovery list-instances --service-id "$RELAY_SERVICE_ID" --region "$AWS_REGION" --query 'Instances[0].Attributes.AWS_INSTANCE_IPV4' --output text)
RELAY_NOISE=$(aws servicediscovery list-instances --service-id "$RELAY_SERVICE_ID" --region "$AWS_REGION" --query 'Instances[0].Attributes.GBN_NOISE_PUBKEY_HEX' --output text)

echo "RELAY_SERVICE_ID=$RELAY_SERVICE_ID"
echo "RELAY_IP=$RELAY_IP"
echo "RELAY_NOISE=$RELAY_NOISE"

echo "\nAttempt 1: run creator container tcp probe to relay:9001"
aws ecs execute-command \
  --cluster "$CLUSTER_ARN" \
  --task "$CREATOR_TASK" \
  --container creator \
  --interactive \
  --command "sh -lc 'which nc >/dev/null 2>&1 && nc -vz -w 3 $RELAY_IP 9001 || echo no_nc_or_connect_failed'" \
  --region "$AWS_REGION" || true