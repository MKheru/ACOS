#!/usr/bin/env python3
"""LLM Proxy — bridges ACOS (QEMU virtio-serial) to Gemini API.

Reads JSON-RPC requests from a virtio-serial chardev (Unix socket),
forwards to Google Gemini API, returns JSON-RPC responses.

Usage:
    python3 llm-proxy.py [--socket /tmp/acos-llm.sock] [--model gemini-2.5-flash]

The proxy listens on a Unix socket that QEMU connects to via:
    -chardev socket,id=llm,path=/tmp/acos-llm.sock,server=on,wait=off
    -device virtio-serial -device virtserialport,chardev=llm,name=llm
"""

import json
import os
import socket
import sys
import time
import urllib.request
import urllib.error
import argparse
import threading

# Load API key
ENV_FILE = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), ".env")
API_KEY = None

def load_api_key():
    global API_KEY
    # Try .env file
    if os.path.exists(ENV_FILE):
        with open(ENV_FILE) as f:
            for line in f:
                if line.startswith("API="):
                    API_KEY = line.strip().split("=", 1)[1]
                    return
    # Try environment
    API_KEY = os.environ.get("GEMINI_API_KEY") or os.environ.get("API")
    if not API_KEY:
        print("ERROR: No API key found. Set API= in .env or GEMINI_API_KEY env var", file=sys.stderr)
        sys.exit(1)


def call_gemini(prompt: str, model: str = "gemini-2.5-flash", max_tokens: int = 256) -> dict:
    """Call Gemini API and return response."""
    url = f"https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent"

    # Build system prompt for ACOS
    system_prompt = (
        "You are the AI brain of ACOS (Agent-Centric Operating System), "
        "a Rust-based micro-kernel OS. You help users interact with the system, "
        "explain commands, and assist with tasks. Keep responses concise and helpful. "
        "You can reference MCP services: mcp://system/info, mcp://process/list, "
        "mcp://file/read, mcp://log/write, mcp://config/get."
    )

    payload = {
        "contents": [
            {"role": "user", "parts": [{"text": prompt}]}
        ],
        "systemInstruction": {"parts": [{"text": system_prompt}]},
        "generationConfig": {
            "maxOutputTokens": max_tokens,
            "temperature": 0.7,
        }
    }

    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(url, data=data, headers={
        "Content-Type": "application/json",
        "x-goog-api-key": API_KEY,  # API key in header, NOT in URL query string
    })

    start = time.time()
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            result = json.loads(resp.read())
        elapsed = time.time() - start

        text = result["candidates"][0]["content"]["parts"][0]["text"]
        # Estimate tokens (rough: ~4 chars per token)
        tokens = len(text) // 4
        return {
            "text": text,
            "tokens_generated": tokens,
            "tokens_per_sec": tokens / elapsed if elapsed > 0 else 0,
            "model": model,
        }
    except Exception as e:
        return {"error": str(e)}


def handle_jsonrpc(request_str: str, model: str) -> str:
    """Process a JSON-RPC request and return a JSON-RPC response."""
    try:
        req = json.loads(request_str)
    except json.JSONDecodeError as e:
        return json.dumps({"jsonrpc": "2.0", "error": {"code": -32700, "message": f"Parse error: {e}"}, "id": None})

    method = req.get("method", "")
    params = req.get("params", {})
    req_id = req.get("id")

    if method == "generate":
        prompt = params.get("prompt", "")
        if not prompt:
            return json.dumps({"jsonrpc": "2.0", "error": {"code": -32602, "message": "missing prompt"}, "id": req_id})
        max_tokens = params.get("max_tokens", 256)
        result = call_gemini(prompt, model=model, max_tokens=max_tokens)
        if "error" in result:
            return json.dumps({"jsonrpc": "2.0", "error": {"code": -32603, "message": result["error"]}, "id": req_id})
        return json.dumps({"jsonrpc": "2.0", "result": result, "id": req_id})

    elif method == "info":
        return json.dumps({"jsonrpc": "2.0", "result": {
            "model_name": model,
            "quantization": "API",
            "ram_mb": 0,
            "tokens_per_sec": 0.0,
            "backend": "gemini-api",
        }, "id": req_id})

    elif method == "stream":
        prompt = params.get("prompt", "")
        if not prompt:
            return json.dumps({"jsonrpc": "2.0", "error": {"code": -32602, "message": "missing prompt"}, "id": req_id})
        result = call_gemini(prompt, model=model, max_tokens=params.get("max_tokens", 256))
        if "error" in result:
            return json.dumps({"jsonrpc": "2.0", "error": {"code": -32603, "message": result["error"]}, "id": req_id})
        # Format as JSONL for streaming compatibility
        lines = []
        words = result["text"].split()
        for i, word in enumerate(words):
            lines.append(json.dumps({"token": word + " ", "index": i}))
        lines.append(json.dumps({"done": True, "total_tokens": len(words)}))
        result["text"] = "\n".join(lines)
        return json.dumps({"jsonrpc": "2.0", "result": result, "id": req_id})

    else:
        return json.dumps({"jsonrpc": "2.0", "error": {"code": -32601, "message": f"unknown method: {method}"}, "id": req_id})


def run_socket_server(socket_path: str, model: str):
    """Listen on TCP for ACOS connections (and Unix socket for local testing)."""
    # TCP server on port 9999 (accessible from QEMU via 10.0.2.2:9999)
    tcp_srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    tcp_srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    tcp_srv.bind(("0.0.0.0", 9999))
    tcp_srv.listen(5)
    print(f"LLM Proxy listening on TCP :9999 (model: {model})")
    print(f"From ACOS: mcp-query llm generate 'your prompt'")
    print(f"ACOS connects to tcp:10.0.2.2:9999")

    while True:
        conn, addr = tcp_srv.accept()
        print(f"[{time.strftime('%H:%M:%S')}] Client connected from {addr}")
        threading.Thread(target=handle_client, args=(conn, model), daemon=True).start()


def handle_client(conn: socket.socket, model: str):
    """Handle a single client connection (QEMU virtio-serial)."""
    buf = b""
    try:
        while True:
            data = conn.recv(65536)
            if not data:
                break
            buf += data

            # Try to parse complete JSON-RPC messages (newline-delimited)
            while b"\n" in buf:
                line, buf = buf.split(b"\n", 1)
                line = line.strip()
                if not line:
                    continue
                request_str = line.decode("utf-8", errors="replace")
                print(f"[{time.strftime('%H:%M:%S')}] Request: {request_str[:100]}...")

                response = handle_jsonrpc(request_str, model)
                print(f"[{time.strftime('%H:%M:%S')}] Response: {response[:100]}...")
                conn.sendall((response + "\n").encode("utf-8"))

            # Also try if buffer itself is a complete JSON object (no newline)
            if buf:
                try:
                    json.loads(buf)
                    request_str = buf.decode("utf-8", errors="replace")
                    buf = b""
                    print(f"[{time.strftime('%H:%M:%S')}] Request: {request_str[:100]}...")
                    response = handle_jsonrpc(request_str, model)
                    print(f"[{time.strftime('%H:%M:%S')}] Response: {response[:100]}...")
                    conn.sendall((response + "\n").encode("utf-8"))
                except json.JSONDecodeError:
                    pass  # Incomplete, wait for more data

    except Exception as e:
        print(f"[{time.strftime('%H:%M:%S')}] Client error: {e}")
    finally:
        conn.close()
        print(f"[{time.strftime('%H:%M:%S')}] Client disconnected")


def main():
    parser = argparse.ArgumentParser(description="ACOS LLM Proxy — bridges QEMU to Gemini API")
    parser.add_argument("--socket", default="/tmp/acos-llm.sock", help="Unix socket path")
    parser.add_argument("--model", default="gemini-2.5-flash", help="Gemini model name")
    parser.add_argument("--test", action="store_true", help="Test mode: read from stdin instead of socket")
    args = parser.parse_args()

    load_api_key()
    print(f"API key loaded ({API_KEY[:10]}...)")

    if args.test:
        # Interactive test mode
        print(f"Test mode — type prompts (model: {args.model}):")
        while True:
            try:
                prompt = input("> ")
            except EOFError:
                break
            result = call_gemini(prompt, model=args.model)
            if "error" in result:
                print(f"ERROR: {result['error']}")
            else:
                print(f"{result['text']}")
                print(f"({result['tokens_generated']} tokens, {result['tokens_per_sec']:.1f} tok/s)")
    else:
        run_socket_server(args.socket, args.model)


if __name__ == "__main__":
    main()
