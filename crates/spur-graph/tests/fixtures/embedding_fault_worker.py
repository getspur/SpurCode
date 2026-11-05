#!/usr/bin/env python3
"""Protocol-only graph test worker. It never loads a native runtime."""
import json
import os
import struct
import sys
import threading
import time

log = os.environ["SPUR_GRAPH_TEST_WORKER_LOG"]
def record(event):
    with open(log, "a", encoding="utf-8") as file:
        file.write(event + "\n")

def send(value):
    data = json.dumps(value, separators=(",", ":")).replace('"NONFINITE_NUMBER"', '1e100').encode()
    sys.stdout.buffer.write(struct.pack(">I", len(data)) + data)
    sys.stdout.buffer.flush()

def recv():
    header = sys.stdin.buffer.read(4)
    if len(header) != 4:
        return None
    size = struct.unpack(">I", header)[0]
    data = sys.stdin.buffer.read(size)
    value = json.loads(data)
    with open(log + ".envelopes", "a", encoding="utf-8") as file:
        file.write(json.dumps({"body_bytes": len(data), "envelope": value}) + "\n")
    return value

def main():
    args = sys.argv[1:]
    identity = {"model": "NomicEmbedTextV15", "dimensions": 768,
                "model_fingerprint": "test-model", "tokenizer_fingerprint": "test-tokenizer",
                "pooling": "mean", "normalization": "unit", "prefix": "legacy_preformatted"}
    if args[0] == "--describe-model":
        identity["model"] = args[1]
        record("describe " + args[1])
        if os.environ.get("SPUR_GRAPH_TEST_WORKER_FAULT") == "descriptor_hang":
            record("descriptor_pid " + str(os.getpid()))
            threading.Event().wait()
        send({"version": 2, "generation": args[3], "request_id": int(args[4]),
              "reply": {"type": "Ready", "identity": identity}})
        return
    record("service")
    count = 0
    while True:
        envelope = recv()
        if envelope is None:
            break
        request = envelope["request"]
        kind = request["type"]
        if kind == "Hello":
            identity = request["identity"]
            record("hello " + identity["model"])
            reply = {"type": "Ready", "identity": identity}
        elif kind == "Shutdown":
            record("shutdown")
            send({"version": 2, "generation": envelope["generation"],
                  "request_id": envelope["request_id"], "reply": {"type": "Closed"}})
            break
        else:
            count += 1
            texts = request["texts"]
            record("embed " + str(len(texts)))
            with open(log + ".inputs", "a", encoding="utf-8") as file:
                file.write(json.dumps(texts) + "\n")
            fault = os.environ.get("SPUR_GRAPH_TEST_WORKER_FAULT", "")
            if fault == "query_hang":
                threading.Event().wait()
            if count == 2 and fault == "second_hang":
                threading.Event().wait()
            if fault == "batch_delay":
                time.sleep(0.2)  # Existing supervisor late-reply fixture duration.
            if count == 2 and fault == "death":
                os.abort()
            if count == 2 and fault == "truncated":
                sys.stdout.buffer.write(struct.pack(">I", 100) + b"{")
                sys.stdout.buffer.flush()
                break
            if count == 2 and fault == "error":
                reply = {"type": "Error", "code": "Unavailable", "message": "injected"}
            else:
                actual = dict(identity)
                if count == 2 and fault == "wrong_identity":
                    actual["model_fingerprint"] = "wrong"
                value = float("nan") if count == 2 and fault == "nonfinite" else None
                reply = {"type": "Embeddings", "identity": actual,
                         "rows": [[value if value is not None else float(len(text))] * 768 for text in texts]}
                if count == 2 and fault == "wrong_rows":
                    reply["rows"].pop()
                if count == 2 and fault == "wrong_dimensions":
                    reply["rows"][0].pop()
                if count == 2 and fault == "wrong_generation":
                    envelope["generation"] = "ffffffff-ffff-ffff-ffff-ffffffffffff"
                if count == 2 and fault == "wrong_request":
                    envelope["request_id"] += 1
                if count == 2 and fault == "nonfinite_number":
                    # Valid JSON numeric syntax that cannot be a finite f32.
                    reply["rows"][0][0] = "NONFINITE_NUMBER"
        send({"version": 2, "generation": envelope["generation"],
              "request_id": envelope["request_id"], "reply": reply})

main()
