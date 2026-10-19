#!/usr/bin/env python3
# A stand-in for volant-agent that speaks the protocol and answers the task named "bad" with the
# frame the FRAME variable asks for, of size N. Every other task gets a small, ordinary result.
import json
import os
import struct
import sys

inp, out = sys.stdin.buffer, sys.stdout.buffer
frame, n = os.environ["FRAME"], int(os.environ["N"])


def read():
    head = inp.read(4)
    if len(head) < 4:
        return None
    return inp.read(struct.unpack(">I", head)[0])


def write(raw):
    out.write(struct.pack(">I", len(raw)))
    out.write(raw)
    out.flush()


def send(obj):
    write(json.dumps(obj).encode())


def result(batch, body):
    head = b'{"type":"task_result","batch":%d,"index":0,"result":' % batch
    write(head + body + b"}")


def hostile(batch):
    if frame == "values":
        result(batch, b'{"changed":false,"x":[' + b"0," * (n - 1) + b"0]}")
    elif frame == "lines":
        result(batch, b'{"changed":false,"rc":0,"stdout":"' + b"\\n" * n + b'","stderr":""}')
    elif frame == "depth":
        result(batch, b'{"changed":false,"x":' + b"[" * n + b"]" * n + b"}")
    elif frame in ("log", "logjunk"):
        junk = b'"index":"x",' if frame == "logjunk" else b""
        write(b'{"type":"log",' + junk + b'"level":"info","message":"' + b"x" * n + b'"}')
        send({"type": "task_result", "batch": batch, "index": 0, "result": {"changed": False, "rc": 0}})
    elif frame == "stdout":
        # n bytes of ordinary lines, 50 bytes each.
        line = "x" * 49 + "\n"
        text = (line * (n // 50)).replace("\n", "\\n")
        result(batch, b'{"changed":true,"rc":0,"stderr":"","stdout":"' + text.encode() + b'"}')
    elif frame == "facts":
        # n packages of seven values each, as package_facts reports them.
        pkg = {"name": "pkg", "version": "1.2.3", "release": "4", "epoch": None, "arch": "x86_64", "source": "rpm"}
        body = json.dumps({"changed": False, "ansible_facts": {"packages": {"p%d" % i: [pkg] for i in range(n)}}})
        result(batch, body.encode())
    else:
        raise SystemExit("unknown FRAME " + frame)


while True:
    raw = read()
    if raw is None:
        break
    msg = json.loads(raw)
    kind = msg.get("type")
    if kind == "hello":
        send({"type": "ready", "protocol": 8, "version": "0.1.0-alpha.8", "arch": "x86_64",
              "interpreters": ["/usr/bin/python3"], "natives": []})
    elif kind == "run_batch":
        for i, task in enumerate(msg["tasks"]):
            if task["args"].get("_raw_params", "").startswith("bad"):
                hostile(msg["id"])
            else:
                send({"type": "task_result", "batch": msg["id"], "index": i,
                      "result": {"changed": True, "rc": 0, "stdout": "fine", "stderr": ""}})
        send({"type": "batch_done", "batch": msg["id"], "outcome": {"kind": "completed"}})
    elif kind in ("has_blob", "put_blob"):
        send({"type": "blob_state", "hash": msg.get("hash"), "present": True})
