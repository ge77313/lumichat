#!/usr/bin/env python3
"""Seed the isolated UI review container with non-production sample data."""

from friend_regression import register, request


register("ui_admin", "UI Admin")
alice = register("ui_alice", "林夏")
bob = register("ui_bob", "陈屿")
alice_token, bob_token = alice["token"], bob["token"]
alice_id, bob_id = alice["user"]["id"], bob["user"]["id"]

friend_request = request("POST", "/friend-requests", alice_token, {"identifier": "ui_bob"})
request("POST", f"/friend-requests/{friend_request['id']}/accept", bob_token)
request("POST", f"/dm/{alice_id}/messages", bob_token, {"body": "下午好，首页的新布局我看到了。文字阅读起来轻松很多。", "file_url": None})
request(
    "POST",
    f"/dm/{bob_id}/messages",
    alice_token,
    {"body": "我把图片也整理成更适合聊天阅读的比例了，手机上会自动铺满可用宽度。", "file_url": "/uploads/demo.jpg"},
)
request(
    "POST",
    f"/dm/{alice_id}/messages",
    bob_token,
    {"body": "> 我把图片也整理成更适合聊天阅读的比例了\n\n这个效果很好，留白也刚好。", "file_url": None},
)
print("SEEDED", alice["user"]["id"], bob_id)
