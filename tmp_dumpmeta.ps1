$env:http_proxy=''
$env:https_proxy=''
$env:HTTP_PROXY=''
$env:HTTPS_PROXY=''
$env:ALL_PROXY=''
$env:all_proxy=''
$cmd = 'sh -lc "echo ''{\"cmd\":\"DumpMetadata\"}'' | nc -w 1 127.0.0.1 5050"'
aws ecs execute-command --cluster gbn-proto-phase1-scale-n100-cluster --task arn:aws:ecs:us-east-1:138472308340:task/gbn-proto-phase1-scale-n100-cluster/7c612a3dd0e6411ab6d1222ad6a89ab2 --container relay --interactive --region us-east-1 --command $cmd
