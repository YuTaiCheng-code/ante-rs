#!/usr/bin/env python3
"""判定器 mock：OpenAI Chat Completions SSE。

- 请求体包含 "act as a strict evaluator" → 视为判定器调用，按 JUDGE 序列返回。
- 其他请求 → 返回一句普通助手文本，代表 agent 回合结束。
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

JUDGES = [
    {"ok": False, "impossible": False, "reason": "no test output in the conversation yet"},
    {"ok": True, "impossible": False, "reason": "the test output shows all tests passing"},
]
state = {"judge": 0}


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("content-length", 0))) or b"{}")
        is_judge = "act as a strict evaluator" in json.dumps(body, ensure_ascii=False)
        if is_judge:
            verdict = JUDGES[min(state["judge"], len(JUDGES) - 1)]
            state["judge"] += 1
            text = json.dumps(verdict)
        else:
            text = "Working on it."
        chunk = {
            "id": "chatcmpl-mock",
            "object": "chat.completion.chunk",
            "choices": [{"index": 0, "delta": {"content": text}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5},
        }
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("cache-control", "no-cache")
        self.send_header("connection", "close")
        self.end_headers()
        self.wfile.write(f"data: {json.dumps(chunk)}\n\ndata: [DONE]\n\n".encode())
        self.wfile.flush()
        print("judge" if is_judge else "turn", file=sys.stderr, flush=True)


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8896
    HTTPServer(("127.0.0.1", port), H).serve_forever()
