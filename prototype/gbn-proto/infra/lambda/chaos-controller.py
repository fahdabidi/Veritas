#!/usr/bin/env python3
"""
GBN Phase 1 Scale Test - Chaos Engine

Subnet-aware churn controller for ECS services.

Invocation modes:
1) Scheduled tick (default): applies churn immediately, then asynchronously
   self-invokes a delayed pass for +30s to approximate 30-second cadence.
2) Delayed pass (event = {"delayed": true}): waits 30s, then applies churn.
"""

import json
import os
import random
import time

import boto3


ecs = boto3.client("ecs")
lambda_client = boto3.client("lambda")


def _env_float(name: str, default: float) -> float:
    try:
        return float(os.environ.get(name, str(default)))
    except (TypeError, ValueError):
        return default


def _targets_for_service(cluster_name: str, service_name: str, churn_rate: float):
    task_arns = ecs.list_tasks(cluster=cluster_name, serviceName=service_name).get("taskArns", [])
    if not task_arns:
        return []

    churn_rate = max(0.0, min(1.0, churn_rate))
    kill_count = int(len(task_arns) * churn_rate)
    if kill_count <= 0:
        return []

    kill_count = min(kill_count, len(task_arns))
    return random.sample(task_arns, k=kill_count)


def _apply_churn(cluster_name: str, hostile_service: str, free_service: str, hostile_rate: float, free_rate: float):
    stopped = {"hostile": [], "free": []}

    for arn in _targets_for_service(cluster_name, hostile_service, hostile_rate):
        ecs.stop_task(cluster=cluster_name, task=arn, reason="ChaosEngine-Hostile")
        stopped["hostile"].append(arn)

    for arn in _targets_for_service(cluster_name, free_service, free_rate):
        ecs.stop_task(cluster=cluster_name, task=arn, reason="ChaosEngine-Free")
        stopped["free"].append(arn)

    return stopped


def handler(event, context):
    event = event or {}

    cluster_name = os.environ["CLUSTER_NAME"]
    hostile_service = os.environ.get("HOSTILE_SERVICE_NAME", "HostileRelayService")
    free_service = os.environ.get("FREE_SERVICE_NAME", "FreeRelayService")
    hostile_rate = _env_float("HOSTILE_CHURN_RATE", 0.4)
    free_rate = _env_float("FREE_CHURN_RATE", 0.2)

    is_delayed = bool(event.get("delayed", False))
    delay_seconds = int(os.environ.get("DELAY_SECONDS", "30"))

    if is_delayed:
        time.sleep(max(0, delay_seconds))

    stopped = _apply_churn(
        cluster_name=cluster_name,
        hostile_service=hostile_service,
        free_service=free_service,
        hostile_rate=hostile_rate,
        free_rate=free_rate,
    )

    if not is_delayed:
        lambda_client.invoke(
            FunctionName=context.invoked_function_arn,
            InvocationType="Event",
            Payload=json.dumps({"delayed": True}).encode("utf-8"),
        )

    return {
        "ok": True,
        "mode": "delayed" if is_delayed else "scheduled",
        "cluster": cluster_name,
        "hostile_stopped": len(stopped["hostile"]),
        "free_stopped": len(stopped["free"]),
        "hostile_rate": hostile_rate,
        "free_rate": free_rate,
    }
