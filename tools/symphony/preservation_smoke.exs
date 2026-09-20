# Host-only integration: real controller removal calls the real Python archiver.
alias SymphonyElixir.{Workflow, Workspace, WorkflowStore}

root = Path.join(System.tmp_dir!(), "preservation-smoke-#{System.unique_integer([:positive])}")
workspace = Path.join(root, "workspaces/GH-smoke")
archive = Path.join(root, "retained")
workflow = Path.join(root, "WORKFLOW.md")
script = Path.join(__DIR__, "preserve_workspace.py")
File.mkdir_p!(workspace)
File.write!(Path.join(workspace, "evidence.txt"), "retained evidence")
File.write!(workflow, """
---
tracker:
  kind: linear
  api_key: synthetic-unused
  project_slug: preservation-fixture
workspace:
  root: #{Path.dirname(workspace)}
hooks:
  before_remove_required: true
  before_remove: python3 #{script} "$PWD" --archive #{archive}
  after_run: python3 #{script} "$PWD" --archive #{archive}
  timeout_ms: 10000
---
Synthetic preservation integration, no tracker or model invocation.
""")

try do
  Workflow.set_workflow_file_path(workflow)
  {:ok, store} = WorkflowStore.start_link()
  # Missing manifest cannot delete the source on ordinary or recorded cleanup.
  {:error, _, _} = Workspace.remove(workspace)
  {:error, _, _} = Workspace.remove_recorded(workspace, nil)
  true = File.exists?(Path.join(workspace, "evidence.txt"))
  manifest = Path.join(workspace, ".symphony-evidence.json")
  sha = :crypto.hash(:sha256, "retained evidence") |> Base.encode16(case: :lower)
  entry = %{path: "evidence.txt", sha256: String.duplicate("0", 64)}
  File.write!(manifest, Jason.encode!(%{schema: "symphony-evidence/v1", files: [entry]}))
  {:error, _, _} = Workspace.remove(workspace)
  true = File.exists?(Path.join(workspace, "evidence.txt"))
  File.write!(manifest, Jason.encode!(%{schema: "symphony-evidence/v1", files: [%{entry | sha256: sha}]}))
  :ok = Workspace.run_after_run_hook(workspace, "GH-smoke")
  [_handoff_receipt] = Path.wildcard(Path.join(archive, "*/receipt.json"))
  true = File.exists?(workspace)
  # Restart the workflow store; the required policy must remain effective.
  GenServer.stop(store)
  {:ok, restarted} = WorkflowStore.start_link()
  {:ok, _} = Workspace.remove_recorded(workspace, nil)
  false = File.exists?(workspace)
  [receipt] = Path.wildcard(Path.join(archive, "*/receipt.json"))
  "retained evidence" = File.read!(Path.join(Path.dirname(receipt), "0000.evidence"))
  GenServer.stop(restarted)
  IO.puts(Jason.encode!(%{status: "PASS", missing_retained: true, mismatch_retained: true,
    restart_policy_preserved: true, host_after_run_archived: true,
    successful_cleanup_retained_evidence: true}))
after
  File.rm_rf!(root)
end
