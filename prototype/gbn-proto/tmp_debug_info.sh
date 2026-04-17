set -euo pipefail
export AWS_REGION=us-east-1
export STACK_NAME=gbn-proto-phase1-scale-n100

CLUSTER_ARN=$(aws cloudformation describe-stack-resources --stack-name "$STACK_NAME" --logical-resource-id ECSCluster --query 'StackResources[0].PhysicalResourceId' --output text --region "$AWS_REGION")
CREATOR_SVC_ARN=$(aws cloudformation describe-stack-resources --stack-name "$STACK_NAME" --logical-resource-id CreatorService --query 'StackResources[0].PhysicalResourceId' --output text --region "$AWS_REGION")
RELAY_SVC_ARN=$(aws cloudformation describe-stack-resources --stack-name "$STACK_NAME" --logical-resource-id FreeRelayService --query 'StackResources[0].PhysicalResourceId' --output text --region "$AWS_REGION")

TASK_DEF_ARN=$(aws ecs describe-services --cluster "$CLUSTER_ARN" --services "$CREATOR_SVC_ARN" --query 'services[0].taskDefinition' --output text --region "$AWS_REGION")
NETWORK_CFG=$(aws ecs describe-services --cluster "$CLUSTER_ARN" --services "$CREATOR_SVC_ARN" --query 'services[0].networkConfiguration' --output json --region "$AWS_REGION")
RELAY_IP=$(aws servicediscovery discover-instances --namespace-name gbn.local --service-name relay --query 'Instances[0].Attributes.AWS_INSTANCE_IPV4' --output text --region "$AWS_REGION")

cat <<"EOF"
echo "CLUSTER=$CLUSTER_ARN"
echo "CREATOR_TD=$TASK_DEF_ARN"
echo "NETWORK_CFG=$NETWORK_CFG"
echo "RELAY_IP=$RELAY_IP"
EOF
EOF
