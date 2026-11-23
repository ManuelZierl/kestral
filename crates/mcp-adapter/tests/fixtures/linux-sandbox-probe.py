import errno
import json
import os
from pathlib import Path
import socket
import sys


def write_denied(path):
    try:
        path.write_text("unexpected")
    except OSError as error:
        return error.errno == errno.EROFS
    return False


def socket_denied(family):
    try:
        with socket.socket(family, socket.SOCK_STREAM):
            return False
    except OSError as error:
        return error.errno == errno.EPERM


request = json.loads(sys.stdin.readline())
payload = Path(os.environ["APP_HOST_PAYLOAD_DIR"])
data = Path(os.environ["APP_HOST_DATA_DIR"])
scratch = Path("/tmp/scratch")
scratch.write_text("ok")
(data / "result").write_text("ok")
print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": {
    "payload_read_only": write_denied(payload / "unexpected"),
    "outside_hidden": not Path(sys.argv[1]).exists(),
    "cwd_is_app_data": Path.cwd() == data,
    "scratch_writable": scratch.read_text() == "ok",
    "environment_cleared": "KESTRAL_TEST_AMBIENT" not in os.environ,
    "tcp_denied": socket_denied(socket.AF_INET),
    "unix_socket_denied": socket_denied(socket.AF_UNIX),
}}), flush=True)
