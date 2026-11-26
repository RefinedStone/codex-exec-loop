"""기술 블로그 자료·리비전 경계를 실제 Git 저장소와 PNG로 검증한다."""
import importlib.util
import io
import json
import os
from io import BytesIO
from pathlib import Path
import subprocess
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

from PIL import Image


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("tech_blog", ROOT / "scripts/tech_blog.py")
worker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(worker)


class BlogWorkflowTest(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        repo = self.root / "repo"
        repo.mkdir()
        self.repo = repo
        self.env = {k: v for k, v in os.environ.items() if not k.upper().startswith("GIT_")}
        self.git("init", "-q")
        (repo / "report.txt").write_text("before\nafter\n", encoding="utf-8")
        self.git("add", "report.txt")
        self.git("-c", "user.name=Tester", "-c", "user.email=t@localhost",
                 "-c", "commit.gpgsign=false", "commit", "-qm", "review handoff")
        self.brief = json.loads((ROOT / "examples/tech-blog/pr-review-handoff/brief.json").read_text("utf-8"))
        self.brief["commit"] = self.git("rev-parse", "HEAD")
        for source in self.brief["sources"]:
            source.update(path="report.txt", lines=[1, 2])
        self.write_brief()
        self.review = self.root / "review.json"

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.repo), *args], text=True, env=self.env).strip()

    def write_brief(self):
        (self.root / "brief.json").write_text(json.dumps(self.brief), "utf-8")

    def run_cli(self, command, *extra):
        import sys
        result = subprocess.run([sys.executable, str(ROOT / "scripts/tech_blog.py"), command,
                                 "--repo", str(self.repo), "--case", str(self.root), *extra],
                                capture_output=True, text=True, env=self.env)
        return result.returncode, json.loads(result.stdout)

    def add_captures(self):
        (self.root / "assets").mkdir()
        for i, figure in enumerate(self.brief["figures"]):
            buffer = BytesIO()
            Image.new("RGB", (5, 5), (40, i, 70)).save(buffer, format="PNG")
            data = buffer.getvalue()
            (self.root / figure["path"]).write_bytes(data)
            figure.update(sha256=worker.digest(data), visibility="public")
        self.write_brief()

    def write_draft(self):
        lines = [f"![{f['alt']}]({f['path']})" for f in self.brief["figures"]]
        (self.root / "draft.md").write_text("\n".join(lines), "utf-8")

    def test_real_git_and_missing_captures(self):
        _, research = self.run_cli("discover", "--term", "review")
        self.assertIn(self.brief["commit"], research["candidates"][0])
        _, output = self.run_cli("packet")
        self.assertEqual("before\nafter", output["packet"]["sources"][0]["text"])
        code, result = self.run_cli("check")
        self.assertEqual((1, "needs_evidence"), (code, result["state"]))
        self.assertEqual(["original", "composer"], [f["figure"] for f in result["findings"]])

    def test_input_errors(self):
        for key in ("kind", "style", "mode"):
            with self.subTest(key=key):
                original = self.brief[key]
                self.brief[key] = "unknown"
                self.write_brief()
                self.assertEqual(2, self.run_cli("check")[0])
                self.brief[key] = original
        self.brief["sources"][0]["lines"] = [1, 99]
        self.write_brief()
        self.assertEqual(2, self.run_cli("check")[0])
        self.brief["sources"][0]["lines"] = [1, 2]
        self.brief["figures"][0]["path"] = "../outside.png"
        self.write_brief()
        self.assertEqual(2, self.run_cli("check")[0])

    def test_images_must_match_public_file_and_draft(self):
        self.add_captures()
        self.assertEqual("awaiting_draft", self.run_cli("check")[1]["state"])
        (self.root / "draft.md").write_text("fixture", "utf-8")
        self.assertEqual("figure_not_in_draft", self.run_cli("check")[1]["findings"][0]["code"])
        self.write_draft()
        fig = self.brief["figures"][0]
        fig["visibility"] = "internal"
        self.write_brief()
        self.assertEqual("public_copy_needed", self.run_cli("check")[1]["findings"][0]["code"])
        fig["visibility"] = "public"
        self.write_brief()
        (self.root / fig["path"]).write_bytes(b"not a PNG")
        self.assertEqual("capture_hash_mismatch", self.run_cli("check")[1]["findings"][0]["code"])
        fig["sha256"] = worker.digest(b"not a PNG")
        self.write_brief()
        self.assertEqual("capture_invalid", self.run_cli("check")[1]["findings"][0]["code"])

    def test_review_must_cover_current_inputs(self):
        self.add_captures()
        self.write_draft()
        _, result = self.run_cli("check")
        self.assertEqual("awaiting_review", result["state"])
        review = {"model": "fixture", "input_sha256": result["input_sha256"], "checks": []}

        def check():
            self.review.write_text(json.dumps(review), "utf-8")
            return self.run_cli("check", "--review", str(self.review))

        self.assertEqual("review_incomplete", check()[1]["findings"][0]["code"])
        for key in ["draft", "claim/transfer", "claim/send", "figure/original", "figure/composer"]:
            review["checks"].append({"id": key, "reason": "fixture observation", "decision": "pass",
                                     "sources": ["handoff-check"]})
        code, approved = check()
        self.assertEqual((0, "reviewed"), (code, approved["state"]))
        review["checks"][1]["decision"] = "revise"
        self.assertEqual("needs_revision", check()[1]["state"])
        with (self.root / "draft.md").open("a", encoding="utf-8") as stream:
            stream.write("\nnew fact")
        self.assertEqual("review_outdated", check()[1]["findings"][0]["code"])

    def test_review_waits_before_starting_a_model(self):
        with patch.object(worker, "run_reviewer", side_effect=AssertionError("unexpected model call")):
            state, status = self.review_cli()
            self.assertEqual((1, "needs_evidence"), (status, state["state"]))
            self.add_captures()
            state, status = self.review_cli()
            self.assertEqual((1, "awaiting_draft"), (status, state["state"]))

    def review_cli(self):
        args = ["tech_blog.py", "review", "--repo", str(self.repo), "--case", str(self.root),
                "--model", "test-model"]
        with patch("sys.argv", args):
            return worker.main()

    def review_checks(self):
        return [
            {"id": name, "reason": "fixture observation", "decision": "pass", "sources": sources}
            for name, sources in [
                ("draft", []), ("claim/transfer", ["review-builder"]),
                ("claim/send", ["handoff-check"]), ("figure/original", []), ("figure/composer", []),
            ]
        ]

    def test_review_sends_verified_bytes_and_returns_reusable_result(self):
        self.add_captures()
        self.write_draft()
        workspaces = []

        def simulate(command, **kwargs):
            work = kwargs["cwd"]
            workspaces.append(work)
            self.assertEqual("exec", command[1])
            self.assertIn("--ephemeral", command)
            self.assertIn("--ignore-user-config", command)
            self.assertEqual("read-only", command[command.index("--sandbox") + 1])
            self.assertEqual(["--", "-"], command[-2:])
            self.assertTrue((work / command[command.index("--output-schema") + 1]).is_file())
            payload = json.loads(kwargs["input"].split(b"\n", 1)[1])
            self.assertEqual("before\nafter", payload["packet"]["sources"][0]["text"])
            names = [command[i + 1] for i, arg in enumerate(command) if arg == "--image"]
            self.assertEqual(len(self.brief["figures"]), len(names))
            self.assertEqual(names, [entry["attachment"] for entry in payload["attachments"]])
            for figure, attachment in zip(self.brief["figures"], payload["attachments"]):
                self.assertEqual(figure["id"], attachment["id"])
                self.assertEqual((self.root / figure["path"]).read_bytes(),
                                 (work / attachment["attachment"]).read_bytes())
            # Extra response fields cannot override the worker's revision or model.
            answer = {"checks": self.review_checks(), "input_sha256": "wrong", "model": "wrong"}
            (work / command[command.index("--output-last-message") + 1]).write_text(json.dumps(answer), "utf-8")
            return 0

        class SimulatedModel:
            """The model process: it gets the prompt on stdin and writes its answer when waited on (#2873)."""

            def __init__(self, command, **kwargs):
                self.command, self.kwargs, self.pid, self.code = command, kwargs, 0, None
                self.stdin = io.BytesIO()
                self.sent = threading.Event()
                self.stdin.close = self.sent.set  # keep the prompt readable after the runner closes stdin

            def wait(self, timeout=None):
                # The runner sends the prompt from another thread; answer only after stdin is closed.
                if not self.sent.wait(timeout):
                    raise subprocess.TimeoutExpired(self.command, timeout)
                if self.code is None:
                    self.code = simulate(self.command, input=self.stdin.getvalue(), cwd=self.kwargs["cwd"])
                return self.code

            def poll(self):
                return self.code

        real_popen = subprocess.Popen

        def popen(command, **kwargs):
            # Git still runs for real; only the model command is simulated.
            return SimulatedModel(command, **kwargs) if command[0] == "fixture-codex" else real_popen(command, **kwargs)

        with patch.object(worker.shutil, "which", return_value="fixture-codex"), \
                patch.object(worker.subprocess, "Popen", side_effect=popen):
            result, code = self.review_cli()
        self.assertEqual((0, "reviewed", "test-model"), (code, result["state"], result["model"]))
        self.assertEqual(1, len(workspaces))
        self.assertFalse(workspaces[0].exists())
        # Windows PowerShell can redirect stdout as UTF-16.
        self.review.write_text(json.dumps(result), "utf-16")
        check_code, checked = self.run_cli("check", "--review", str(self.review))
        self.assertEqual((0, "reviewed"), (check_code, checked["state"]))

    def test_cancel_stops_a_model_that_never_reads_its_prompt(self):
        class StalledModel:
            """A model whose stdin pipe is full: writing blocks until the process is killed (#2873)."""

            def __init__(self):
                self.pid, self.killed = 0, threading.Event()
                self.stdin = self

            def write(self, _):
                self.killed.wait(5)
                raise BrokenPipeError

            def close(self):
                pass

            def wait(self, timeout=None):
                if self.killed.wait(timeout):
                    return 1
                raise subprocess.TimeoutExpired("fixture-codex", timeout)

            def poll(self):
                return 1 if self.killed.is_set() else None

        model = StalledModel()
        started = time.monotonic()
        with patch.object(worker, "STOP_CHECK", lambda: True), \
                patch.object(worker, "kill_process_tree", lambda process: process.killed.set()), \
                self.assertRaises(worker.ReviewerError) as raised:
            worker.wait_for_model(model, b"x" * 200_000)
        self.assertEqual("cancelled", raised.exception.code)
        self.assertLess(time.monotonic() - started, 2)

    def test_draft_change_during_review_invalidates_result(self):
        self.add_captures()
        self.write_draft()

        def edit_draft(*_):
            with (self.root / "draft.md").open("a", encoding="utf-8") as out:
                out.write("\nchanged before result")
            return self.review_checks()

        with patch.object(worker, "run_reviewer", side_effect=edit_draft):
            result, code = self.review_cli()
        self.assertEqual((1, "awaiting_review"), (code, result["state"]))
        self.assertEqual([{"code": "review_outdated"}], result["findings"])
        self.assertNotIn("checks", result)

    def test_reviewer_errors_and_incomplete_response_do_not_pass(self):
        self.add_captures()
        self.write_draft()
        with patch.object(worker.shutil, "which", return_value=None), self.assertRaises(worker.ReviewerError):
            self.review_cli()
        with patch.object(worker, "run_reviewer", return_value=[{"id": "draft"}]):
            result, code = self.review_cli()
        self.assertEqual((1, "awaiting_review"), (code, result["state"]))
        self.assertEqual([{"code": "review_incomplete"}], result["findings"])
        with patch.object(worker, "run_reviewer", return_value=42), self.assertRaises(worker.ReviewerError):
            self.review_cli()


if __name__ == "__main__":
    unittest.main()
