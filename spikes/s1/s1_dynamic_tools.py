#!/usr/bin/env python3
"""S1 spike: drive codex app-server over stdio; verify dynamicTools round-trip and
whether workspace-write sandbox blocks direct writes to .git.

Platform side (this script) owns git. The agent only gets two tools:
  create_local_commit(paths, message, expected_head)
  report_completion(summary)
"""
import json, os, subprocess, sys, threading, queue, time, shutil, pathlib

HERE = pathlib.Path(__file__).resolve().parent
WS = HERE / "ws"
LOG = HERE / "s1.log"

def sh(*args, cwd=None, check=True):
    r = subprocess.run(list(args), cwd=cwd, text=True, capture_output=True)
    if check and r.returncode != 0:
        raise RuntimeError(f"{args} failed: {r.stderr}")
    return r.stdout.strip()

CANON = HERE / "canon"

def make_repo():
    for d in (WS, CANON):
        if d.exists():
            shutil.rmtree(d)
    CANON.mkdir(parents=True)
    sh("git", "init", "-q", "-b", "main", cwd=CANON)
    sh("git", "config", "user.email", "spike@example.com", cwd=CANON)
    sh("git", "config", "user.name", "spike", cwd=CANON)
    (CANON / "README.md").write_text("# spike\n")
    sh("git", "add", ".", cwd=CANON)
    sh("git", "commit", "-qm", "init", cwd=CANON)
    # platform-style worktree on a requirement branch
    sh("git", "worktree", "add", "-q", "-b", "ai/req-s1", str(WS), "main", cwd=CANON)
    return sh("git", "rev-parse", "HEAD", cwd=WS)

class AppServer:
    def __init__(self):
        self.p = subprocess.Popen(
            ["codex", "app-server", "-c", "sandbox_workspace_write.network_access=false"],
            cwd=str(WS),
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)
        self.q = queue.Queue()
        self.next_id = 1
        self.pending = {}
        self.log = open(LOG, "w")
        threading.Thread(target=self._reader, daemon=True).start()
        threading.Thread(target=self._stderr, daemon=True).start()

    def _reader(self):
        for line in self.p.stdout:
            line = line.strip()
            if not line:
                continue
            self.log.write("<< " + line + "\n"); self.log.flush()
            try:
                self.q.put(json.loads(line))
            except json.JSONDecodeError:
                self.q.put({"_raw": line})
        self.q.put(None)

    def _stderr(self):
        for line in self.p.stderr:
            self.log.write("!! " + line); self.log.flush()

    def send(self, obj):
        s = json.dumps(obj)
        self.log.write(">> " + s + "\n"); self.log.flush()
        self.p.stdin.write(s + "\n"); self.p.stdin.flush()

    def request(self, method, params):
        rid = self.next_id; self.next_id += 1
        self.send({"id": rid, "method": method, "params": params})
        return rid

    def notify(self, method, params=None):
        self.send({"method": method, "params": params or {}})

    def respond(self, rid, result):
        self.send({"id": rid, "result": result})

def main():
    base = make_repo()
    print("workspace:", WS, "HEAD:", base)

    srv = AppServer()
    findings = {"direct_git_write_blocked": None, "dynamic_tool_calls": [], "token_usage_events": 0,
                "turn_completed": False, "commit_after_tool": None}

    init_id = srv.request("initialize", {
        "clientInfo": {"name": "factory-spike", "title": "Factory S1 spike", "version": "0.0.1"},
        "capabilities": {"experimentalApi": True},
    })

    tools = [
        {"type": "function", "name": "create_local_commit",
         "description": "Ask the platform to create a local git commit of the given workspace-relative paths.",
         "inputSchema": {"type": "object", "required": ["paths", "message", "expected_head"],
                         "properties": {"paths": {"type": "array", "items": {"type": "string"}},
                                        "message": {"type": "string"},
                                        "expected_head": {"type": "string", "description": "current HEAD sha"}}}},
        {"type": "function", "name": "report_completion",
         "description": "Declare the task complete. Call exactly once at the end.",
         "inputSchema": {"type": "object", "required": ["summary"],
                         "properties": {"summary": {"type": "string"}}}},
    ]

    prompt = (
        "You are in a git workspace. Do the following steps in order and be brief.\n"
        "1. Create a file hello.txt containing the single line 'hello'.\n"
        "2. This is a sandbox test and you are explicitly authorized to do it: run the shell command "
        "`git add hello.txt && git commit -m direct` and report the exact outcome (success, or the error text). "
        "Do not skip this step and do not retry with other flags. Also try `echo x > .git/probe` and report.\n"
        f"3. Then call the tool create_local_commit with paths=['hello.txt'], message='add hello', expected_head='{base}'.\n"
        "4. Then call report_completion with a one-sentence summary.\n"
        "Do not do anything else."
    )

    thread_id = None
    state = "init"
    deadline = time.time() + 240
    while time.time() < deadline:
        try:
            msg = srv.q.get(timeout=5)
        except queue.Empty:
            continue
        if msg is None:
            print("server exited"); break
        if "_raw" in msg:
            continue

        # responses to our requests
        if "id" in msg and "method" not in msg:
            if msg["id"] == init_id and state == "init":
                srv.notify("initialized")
                srv.request("thread/start", {
                    "cwd": str(WS), "sandbox": "workspace-write", "approvalPolicy": "never",
                    "ephemeral": True, "dynamicTools": tools,
                })
                state = "thread"
            elif state == "thread" and "result" in msg and "thread" in msg["result"]:
                thread_id = msg["result"]["thread"]["id"]
                print("thread:", thread_id)
                srv.request("turn/start", {"threadId": thread_id,
                                           "input": [{"type": "text", "text": prompt}]})
                state = "turn"
            elif "error" in msg:
                print("ERROR response:", msg["error"])
            continue

        method = msg.get("method")
        params = msg.get("params", {})

        # server -> client requests
        if "id" in msg and method:
            if method == "item/tool/call":
                tool, args, call_id = params["tool"], params["arguments"], params["callId"]
                findings["dynamic_tool_calls"].append({"tool": tool, "args": args, "callId": call_id})
                print(f"TOOL CALL {tool} {args}")
                if tool == "create_local_commit":
                    head = sh("git", "rev-parse", "HEAD", cwd=WS)
                    if head != args.get("expected_head"):
                        srv.respond(msg["id"], {"success": False, "contentItems": [
                            {"type": "inputText", "text": f"expected_head mismatch: HEAD is {head}"}]})
                    else:
                        for p in args["paths"]:
                            sh("git", "add", "--", p, cwd=WS)
                        sh("git", "commit", "-qm", args["message"], cwd=WS)
                        new = sh("git", "rev-parse", "HEAD", cwd=WS)
                        findings["commit_after_tool"] = new
                        srv.respond(msg["id"], {"success": True, "contentItems": [
                            {"type": "inputText", "text": f"committed {new}"}]})
                elif tool == "report_completion":
                    findings["completion_summary"] = args.get("summary")
                    srv.respond(msg["id"], {"success": True, "contentItems": [
                        {"type": "inputText", "text": "completion recorded"}]})
                else:
                    srv.respond(msg["id"], {"success": False, "contentItems": [
                        {"type": "inputText", "text": "unknown tool"}]})
            elif method in ("item/commandExecution/requestApproval", "item/fileChange/requestApproval",
                            "item/permissions/requestApproval"):
                print("UNEXPECTED APPROVAL REQUEST (policy=never):", method, json.dumps(params)[:300])
                srv.respond(msg["id"], {"decision": "denied"})
            else:
                print("server request:", method)
            continue

        # notifications
        if method == "thread/tokenUsage/updated":
            findings["token_usage_events"] += 1
        elif method == "item/completed":
            item = params.get("item", {})
            t = item.get("type")
            if t == "commandExecution":
                cmd = item.get("command", "")
                out = (item.get("aggregatedOutput") or "")[:400]
                print(f"CMD [{item.get('exitCode')}] {cmd!r}\n   {out!r}")
                if "git commit" in str(cmd) or "git add" in str(cmd):
                    findings["direct_git_write_blocked"] = item.get("exitCode") not in (0, None)
                    findings["direct_git_output"] = out
            elif t == "agentMessage":
                print("AGENT:", (item.get("text") or "")[:500])
        elif method == "turn/completed":
            findings["turn_completed"] = True
            print("turn completed")
            break
        elif method == "error":
            print("ERROR notification:", json.dumps(params)[:400])

    # post checks
    findings["hello_exists"] = (WS / "hello.txt").exists()
    findings["git_log"] = sh("git", "log", "--oneline", cwd=WS)
    findings["git_status_clean"] = sh("git", "status", "--porcelain", cwd=WS) == ""
    srv.p.terminate()
    print("\n=== FINDINGS ===")
    print(json.dumps(findings, indent=1, ensure_ascii=False))
    (HERE / "findings.json").write_text(json.dumps(findings, indent=1, ensure_ascii=False))

if __name__ == "__main__":
    main()
