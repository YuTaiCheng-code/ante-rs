import http.server,json,sys
class H(http.server.BaseHTTPRequestHandler):
    protocol_version='HTTP/1.1'
    def do_POST(self):
        n=int(self.headers.get('content-length',0)); b=json.loads(self.rfile.read(n) or b'{}')
        open('req_tool.log','a').write(json.dumps(b,ensure_ascii=False)+'\n')
        self.send_response(200); self.send_header('Content-Type','text/event-stream'); self.send_header('Connection','close'); self.end_headers()
        has_tool=any(m.get('role')=='tool' for m in b.get('messages',[]))
        if has_tool:
            last=[x for x in b.get('messages',[]) if x.get('role')=='tool'][-1]['content'][:120]
            for c in [{"choices":[{"index":0,"delta":{"content":"tool-said: "+last}}]},{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":20,"completion_tokens":2}}]:
                self.wfile.write(b'data: '+json.dumps(c).encode()+b'\n\n')
        else:
            for c in [{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"Bash","arguments":""}}]}}]},
                      {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"command\":\"echo hi\"}"}}]}}]},
                      {"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":15,"completion_tokens":5}}]:
                self.wfile.write(b'data: '+json.dumps(c).encode()+b'\n\n')
        self.wfile.write(b'data: [DONE]\n\n'); self.wfile.flush()
    def log_message(self,*a):pass
http.server.ThreadingHTTPServer(('127.0.0.1',int(sys.argv[1]) if len(sys.argv)>1 else 8894),H).serve_forever()
