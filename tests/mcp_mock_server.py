#!/usr/bin/env python3
"""Minimal MCP stdio server used by the Rust integration test."""
import json
import sys

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    msg = json.loads(line)
    method = msg.get("method")
    req_id = msg.get("id")
    if method == "initialize":
        out = {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "mock", "version": "0.0.1"},
            },
        }
    elif method == "notifications/initialized":
        continue
    elif method == "tools/list":
        out = {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {
                "tools": [
                    {
                        "name": "echo",
                        "description": "Echo the supplied text.",
                        "inputSchema": {
                            "type": "object",
                            "properties": {"text": {"type": "string"}},
                            "required": ["text"],
                        },
                    }
                ]
            },
        }
    elif method == "tools/call":
        text = msg.get("params", {}).get("arguments", {}).get("text", "")
        out = {
            "jsonrpc": "2.0",
            "id": req_id,
            "result": {"content": [{"type": "text", "text": "echo:" + text}], "isError": False},
        }
    else:
        if req_id is None:
            continue
        out = {"jsonrpc": "2.0", "id": req_id, "error": {"code": -32601, "message": "unknown method"}}
    print(json.dumps(out), flush=True)
