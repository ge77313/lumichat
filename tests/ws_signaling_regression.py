#!/usr/bin/env python3
"""Verify WebSocket call signaling is delivered only between friends."""

import base64
import json
import os
import socket
import struct
import sys
import urllib.error
import urllib.request


HOST = "127.0.0.1"
PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 18084
BASE = f"http://{HOST}:{PORT}/api"


def api(method, path, token=None, payload=None):
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    data = json.dumps(payload).encode() if payload is not None else None
    with urllib.request.urlopen(urllib.request.Request(BASE + path, data=data, headers=headers, method=method)) as response:
        return json.loads(response.read())


def register(username):
    return api("POST", "/register", payload={"username": username, "password": "test-pass-123", "display_name": username.title()})


def recv_exact(sock, size):
    data = b""
    while len(data) < size:
        chunk = sock.recv(size - len(data))
        if not chunk:
            raise EOFError("socket closed")
        data += chunk
    return data


def recv_frame(sock):
    first, second = recv_exact(sock, 2)
    length = second & 0x7F
    if length == 126:
        length = struct.unpack("!H", recv_exact(sock, 2))[0]
    elif length == 127:
        length = struct.unpack("!Q", recv_exact(sock, 8))[0]
    if second & 0x80:
        mask = recv_exact(sock, 4)
        payload = bytes(value ^ mask[index % 4] for index, value in enumerate(recv_exact(sock, length)))
    else:
        payload = recv_exact(sock, length)
    return first & 0x0F, payload


def send_text(sock, value):
    payload = json.dumps(value).encode()
    mask = os.urandom(4)
    header = bytes([0x81])
    if len(payload) < 126:
        header += bytes([0x80 | len(payload)])
    else:
        header += bytes([0x80 | 126]) + struct.pack("!H", len(payload))
    sock.sendall(header + mask + bytes(value ^ mask[index % 4] for index, value in enumerate(payload)))


def connect(token):
    sock = socket.create_connection((HOST, PORT), timeout=3)
    key = base64.b64encode(os.urandom(16)).decode()
    request = (
        f"GET /api/ws?token={token} HTTP/1.1\r\nHost: {HOST}:{PORT}\r\nUpgrade: websocket\r\n"
        f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    )
    sock.sendall(request.encode())
    response = b""
    while b"\r\n\r\n" not in response:
        response += sock.recv(4096)
    assert response.startswith(b"HTTP/1.1 101"), response
    opcode, payload = recv_frame(sock)
    assert opcode == 1 and json.loads(payload)["type"] == "ready"
    return sock


def main():
    register("signal_admin")
    a, b, c = register("signal_alice"), register("signal_bob"), register("signal_carol")
    ta, tb, tc = a["token"], b["token"], c["token"]
    bid, cid = b["user"]["id"], c["user"]["id"]
    friend_request = api("POST", "/friend-requests", ta, {"identifier": "signal_bob"})
    api("POST", f"/friend-requests/{friend_request['id']}/accept", tb)

    sender, friend, stranger = connect(ta), connect(tb), connect(tc)
    try:
        send_text(sender, {"type": "call_offer", "to_user_id": bid, "mode": "video", "sdp": {"type": "offer", "sdp": "test"}})
        friend.settimeout(3)
        opcode, payload = recv_frame(friend)
        delivered = json.loads(payload)
        assert opcode == 1 and delivered["type"] == "call_offer" and delivered["from"]["id"] == a["user"]["id"]

        send_text(sender, {"type": "call_offer", "to_user_id": cid, "mode": "video", "sdp": {"type": "offer", "sdp": "must-not-arrive"}})
        stranger.settimeout(1)
        try:
            recv_frame(stranger)
            raise AssertionError("nonfriend received call signaling")
        except socket.timeout:
            pass
    finally:
        sender.close()
        friend.close()
        stranger.close()
    print(json.dumps({"status": "ok", "friend_video_signal": "delivered", "nonfriend_video_signal": "blocked"}))


if __name__ == "__main__":
    main()
