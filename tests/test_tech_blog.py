"""기술 블로그 자료·리비전 경계를 실제 Git 저장소와 PNG로 검증한다."""
import importlib.util
import json
import os
from io import BytesIO
from pathlib import Path
import subprocess
import tempfile
import unittest

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


if __name__ == "__main__":
    unittest.main()
