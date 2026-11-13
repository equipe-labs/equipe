# Send one request to the equipe desktop helper's socket, the way the daemon does.
import json, os, socket, sys
home = os.environ["EQUIPE_HOME"]
token = open(os.path.join(home, "desktop", "token")).read().strip()
tool = sys.argv[1]
args = json.loads(sys.argv[2]) if len(sys.argv) > 2 else {}
s = socket.socket(socket.AF_UNIX)
s.settimeout(90)
s.connect(os.path.join(home, "desktop", "helper.sock"))
s.sendall((json.dumps({"token": token, "member": "tester", "tool": tool, "arguments": args}) + "\n").encode())
buf = b""
while not buf.endswith(b"\n"):
    chunk = s.recv(65536)
    if not chunk:
        break
    buf += chunk
r = json.loads(buf)
print(("ERROR " if r.get("is_error") else "OK ") + r.get("text", ""))
