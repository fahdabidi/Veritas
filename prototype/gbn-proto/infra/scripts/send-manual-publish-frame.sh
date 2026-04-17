#!/usr/bin/env bash

# Send a single framed packet to the publisher mpub-receiver (port 7001).
# Usage:
#   bash send-manual-publish-frame.sh \
#     --host 10.0.3.10 --port 7001 --raw-hex deadbeef...
#
# Optional:
#   --json '{"session_id":[...]}'
#   --json-file /path/to/file.json
#   --gbninit             prepend GBNINIT magic prefix to JSON payload
#
# The receiver expects: [4-byte LE length][payload-bytes]
# Payload can be arbitrary bytes for transport sanity checks, or valid
# Publisher payloads (UploadSessionInit/chunk JSON) for protocol-path checks.

set -euo pipefail

HOST="${GBN_PUBLISHER_HOST:-}"
PORT="${GBN_PUBLISHER_PORT:-7001}"
RAW_HEX=""
JSON_TEXT=""
JSON_FILE=""
GBNINIT=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --host)
      HOST="$2"
      shift 2
      ;;
    --port)
      PORT="$2"
      shift 2
      ;;
    --raw-hex)
      RAW_HEX="$2"
      shift 2
      ;;
    --json)
      JSON_TEXT="$2"
      shift 2
      ;;
    --json-file)
      JSON_FILE="$2"
      shift 2
      ;;
    --gbninit)
      GBNINIT=1
      shift
      ;;
    *)
      echo "ERROR: unknown arg $1"
      echo "Usage: $0 --host <ip> [--port 7001] [--raw-hex <hex>] [--json '<json>'] [--json-file <file>] [--gbninit]"
      exit 1
      ;;
  esac
done

if [[ -z "$HOST" ]]; then
  echo "ERROR: --host is required (or set GBN_PUBLISHER_HOST env var)."
  exit 1
fi

PAYLOAD_SOURCE="text"
if [[ -n "$JSON_FILE" ]]; then
  if [[ ! -f "$JSON_FILE" ]]; then
    echo "ERROR: JSON file not found: $JSON_FILE"
    exit 1
  fi
  PAYLOAD_SOURCE="json-file"
elif [[ -n "$JSON_TEXT" ]]; then
  PAYLOAD_SOURCE="json"
elif [[ -n "$RAW_HEX" ]]; then
  PAYLOAD_SOURCE="hex"
fi

python3 - "$HOST" "$PORT" "$RAW_HEX" "$JSON_TEXT" "$JSON_FILE" "$GBNINIT" <<'PY'
import os
import socket
import struct
import sys

host, port, raw_hex, json_text, json_file, gbninit_flag = sys.argv[1:7]
port = int(port)
gbninit = int(gbninit_flag) == 1

payload = b""
if json_file:
    with open(json_file, "rb") as f:
        payload = f.read()
elif json_text:
    payload = json_text.encode("utf-8")
elif raw_hex:
    payload = bytes.fromhex(raw_hex.replace(" ", ""))
else:
    payload = b"gbn-manual-smoke-test"

if gbninit:
    payload = b"GBNINIT" + payload

frame = struct.pack("<I", len(payload)) + payload
with socket.create_connection((host, port), timeout=5) as sock:
    sock.sendall(frame)

print(f"sent {len(payload)} bytes to {host}:{port} (framed length {len(frame)} bytes, gbninit={gbninit}, source={ 'json-file' if json_file else 'json' if json_text else 'raw-hex' if raw_hex else 'text' })")
PY

