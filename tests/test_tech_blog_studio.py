import json
import os
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import tech_blog_studio as studio


def draft_answer(text="# 키 생성 경합 막기\n본문"):
    return {"title": "키 생성 경합 막기", "slug": "key-race", "style": "PROBLEM_SOLVING", "draft": text, "reply": "초안을 썼습니다.",
            "claims": [{"id": "c1", "text": "키를 한 번만 만든다", "sources": ["s1"]}]}


def review_answer(decision):
    return {"checks": [{"id": "draft", "decision": decision, "reason": "글 전체", "sources": []},
                       {"id": "claim/c1", "decision": decision, "reason": "근거와 대조", "sources": ["s1"]}]}


def process_exists(pid):
    if sys.platform == "win32":
        output = subprocess.run(["tasklist", "/FI", f"PID eq {pid}", "/NH"], capture_output=True, text=True,
                                creationflags=0x08000000).stdout
        return str(pid) in output
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    return True


class FakeModel:
    """스키마 파일 이름으로 집필과 검수를 가른다. 받은 입력을 기록한다."""

    def __init__(self, *reviews, fail_on=None, text=None):
        self.reviews = list(reviews)
        self.fail_on = fail_on
        self.text = text
        self.prompts = []

    def __call__(self, model, prompt, schema, images):
        assert model is None, "자동 블로그는 모델 ID 를 넘기지 않는다"
        kind = "write" if "write" in Path(schema).name else "review"
        self.prompts.append((kind, prompt.decode()))
        if kind == self.fail_on:
            raise studio.blog.ReviewerError("boom", "model_timeout")
        if kind == "write":
            return draft_answer(self.text) if self.text else draft_answer()
        return review_answer(self.reviews.pop(0))


class StudioTest(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        self.runtime = root / "runtime"
        self.runtime.mkdir()
        self.repo = root / "repo"
        self.repo.mkdir()
        self.env = {k: v for k, v in os.environ.items() if not k.upper().startswith("GIT_")}
        self.git("init", "-q")
        for name in ("first", "second", "third"):
            self.commit(name, f"{name} line\n")
        self.host = self.studio("first-kernel")
        self.host.request("configure", {"enabled": True})

    def studio(self, owner):
        host = studio.Studio(self.runtime, self.repo, owner)
        self.addCleanup(host.db.close)
        return host

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.repo), "-c", "core.fsmonitor=false", *args],
                                       text=True, env=self.env).strip()

    def commit(self, name, text):
        (self.repo / f"{name}.txt").write_text(text, encoding="utf-8")
        self.git("add", f"{name}.txt")
        self.git("-c", "user.name=Tester", "-c", "user.email=t@localhost", "-c", "commit.gpgsign=false",
                 "commit", "-qm", f"feat: {name}")
        return self.git("rev-parse", "HEAD")

    def generate(self, **extra):
        targets = self.host.request("targets", {})
        return self.host.request("generate", {"commits": targets["selected"], **extra})

    def run_all(self, a, model, host=None):
        host = host or self.host
        with patch.object(studio.blog, "run_codex_json", model):
            while host.execute_job(a["id"], a["job"]["id"])["more"]:
                pass
        return host.request("open", {"id": a["id"]})

    def test_one_press_produces_a_reviewed_article_without_figures(self):
        a = self.generate(topic="키 생성")
        self.assertEqual(("running", "collect"), (a["state"], a["stage"]))
        model = FakeModel("pass")
        result = self.run_all(a, model)
        self.assertEqual(("reviewed", None, None), (result["state"], result["stage"], result["job"]))
        self.assertEqual(2, result["calls"])
        self.assertEqual("키 생성 경합 막기", result["title"])
        self.assertEqual(3, len(result["evidence"]["commits"]))
        self.assertEqual(["write", "review"], [kind for kind, _ in model.prompts])
        self.assertIn("feat: third", model.prompts[0][1])
        self.assertIn("+third line", model.prompts[0][1])

    def test_the_writing_rules_come_from_the_skill_document(self):
        # #2872: no prompt string in Python; the skill body leads the writer input.
        self.assertFalse(hasattr(studio, "WRITE_PROMPT"))
        skill = (Path(__file__).resolve().parents[1] / "assets/app-server/skills/akra-tech-blog/SKILL.md").read_text("utf-8")
        model = FakeModel("pass")
        self.run_all(self.generate(), model)
        kind, prompt = model.prompts[0]
        self.assertEqual(kind, "write")
        self.assertTrue(prompt.startswith(skill.rstrip("\n") + "\n"))
        self.assertIn('"sources"', prompt[len(skill):])

    def test_findings_trigger_one_automatic_rewrite(self):
        model = FakeModel("revise", "revise")
        result = self.run_all(self.generate(), model)
        self.assertEqual(["write", "review", "write", "review"], [kind for kind, _ in model.prompts])
        self.assertIn("근거와 대조", model.prompts[2][1])
        self.assertEqual(("findings", 4, 1), (result["state"], result["calls"], result["attempt"]))
        self.assertEqual(2, len(result["review"]["findings"]))

    def test_failure_stops_and_resume_does_not_repeat_finished_stages(self):
        a = self.generate()
        result = self.run_all(a, FakeModel(fail_on="write"))
        self.assertEqual(("stopped", "write"), (result["state"], result["stage"]))
        self.assertEqual("model_timeout", result["stopped"]["code"])
        result = self.host.request("save", {"id": a["id"], "revision": result["revision"], "draft": "사람이 쓴 시작"})
        self.assertEqual("stopped", result["state"])
        resumed = self.host.request("resume", {"id": a["id"], "revision": result["revision"]})
        model = FakeModel("pass")
        result = self.run_all(resumed, model)
        self.assertEqual("reviewed", result["state"])
        self.assertEqual(["write", "review"], [kind for kind, _ in model.prompts])

    def test_cancel_and_restart_stop_the_job_and_late_results_are_ignored(self):
        a = self.generate()
        self.assertTrue(self.host.execute_job(a["id"], a["job"]["id"])["more"])
        stopped = self.host.request("cancel", {"id": a["id"]})
        self.assertEqual(("stopped", "cancelled", "write"), (stopped["state"], stopped["stopped"]["reason"], stopped["stage"]))
        self.assertEqual({"more": False}, self.host.execute_job(a["id"], a["job"]["id"]))
        b = self.run_all(self.host.request("resume", {"id": a["id"], "revision": stopped["revision"]}), FakeModel("pass"))
        self.assertEqual("reviewed", b["state"])
        commit = [self.host.request("targets", {})["commits"][0]["sha"]]
        c = self.host.request("generate", {"commits": commit})
        with self.assertRaisesRegex(ValueError, "다른 글을 만드는 중"):
            self.host.request("generate", {"commits": commit})
        after = self.studio("next-kernel").request("open", {"id": c["id"]})
        self.assertEqual(("stopped", "interrupted"), (after["state"], after["stopped"]["reason"]))

    def test_cancel_ends_the_running_model_process_within_a_second(self):
        # #2873: a fake model writes its pid and sleeps; cancel from another request must end it.
        folder = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: __import__("shutil").rmtree(folder, ignore_errors=True))
        model = folder / "slow_model.py"
        pid_file = folder / "model.pid"
        model.write_text(
            "import os, sys, time\n"
            f"open({str(pid_file)!r}, 'w').write(str(os.getpid()))\n"
            "sys.stdin.read()\n"
            "time.sleep(60)\n",
            encoding="utf-8",
        )
        a = self.generate()
        self.assertTrue(self.host.execute_job(a["id"], a["job"]["id"])["more"])  # collect
        cancel = {}

        def cancel_when_the_model_runs():
            deadline = time.monotonic() + 20
            while not pid_file.exists() and time.monotonic() < deadline:
                time.sleep(0.05)
            if pid_file.exists():
                other = studio.Studio(self.runtime, self.repo, "first-kernel")  # same kernel, another request
                try:
                    cancel["stopped"] = other.request("cancel", {"id": a["id"]})
                    cancel["at"] = time.monotonic()
                finally:
                    other.db.close()

        canceller = threading.Thread(target=cancel_when_the_model_runs)
        with patch.object(studio.blog, "MODEL_COMMAND", [sys.executable, str(model)]):
            canceller.start()
            result = self.host.execute_job(a["id"], a["job"]["id"])
            finished_at = time.monotonic()
        canceller.join(timeout=10)
        self.assertTrue(pid_file.exists(), "the fake model did not start")
        pid = int(pid_file.read_text())
        stopped = cancel["stopped"]
        self.assertLess(finished_at - cancel["at"], 1.5)
        self.assertEqual({"more": False}, result)
        self.assertFalse(process_exists(pid), "the model process must be gone after cancel")
        self.assertEqual(("stopped", "cancelled", "write"), (stopped["state"], stopped["stopped"]["reason"], stopped["stage"]))
        # Finished stages stay and resume works.
        b = self.run_all(self.host.request("resume", {"id": a["id"], "revision": stopped["revision"]}), FakeModel("pass"))
        self.assertEqual("reviewed", b["state"])

    def test_targets_select_only_commits_after_the_last_article(self):
        first = self.host.request("targets", {})
        self.assertTrue(first["first"])
        self.assertEqual(3, len(first["selected"]))
        self.run_all(self.generate(), FakeModel("pass"))
        empty = self.host.request("targets", {})
        self.assertEqual(([], 0, False), (empty["selected"], empty["fresh"], empty["first"]))
        new = self.commit("fourth", "fourth\n")
        self.assertEqual([new], self.host.request("targets", {})["selected"])
        with self.assertRaisesRegex(ValueError, "30개"):
            self.host.request("generate", {"commits": [f"{n:040x}" for n in range(31)]})
        with self.assertRaisesRegex(ValueError, "대상 커밋"):
            self.host.request("generate", {"commits": ["f" * 40]})

    def test_invalid_model_replies_are_named_and_other_errors_keep_their_message(self):
        bad = lambda model, prompt, schema, images: {**draft_answer(), "claims": [{"id": "c1", "text": "x", "sources": ["s9"]}]}
        result = self.run_all(self.generate(), bad)
        self.assertEqual("model_reply_invalid", result["stopped"]["code"])
        with patch.object(studio, "evidence", side_effect=KeyError("broken")):
            other = self.host.request("resume", {"id": result["id"], "revision": result["revision"]})
            self.host.execute_job(other["id"], other["job"]["id"])
        stopped = self.host.request("open", {"id": result["id"]})
        self.assertEqual("worker_failed", stopped["stopped"]["code"])
        self.assertIn("broken", stopped["stopped"]["message"])

    def test_finished_and_stopped_articles_are_unread_until_seen(self):
        a = self.run_all(self.generate(), FakeModel("pass"))
        self.assertTrue(a["unread"])
        self.assertTrue(self.host.request("list", {})[0]["unread"])
        self.assertFalse(self.host.request("seen", {"id": a["id"]})["unread"])
        job = self.host.request("review", {"id": a["id"], "revision": self.host.request("open", {"id": a["id"]})["revision"]})
        self.assertFalse(job["unread"])
        self.assertFalse(self.host.request("cancel", {"id": a["id"]})["unread"])
        b = self.host.request("generate", {"commits": [self.host.request("targets", {})["commits"][0]["sha"]]})
        stopped = self.run_all(b, FakeModel(fail_on="write"))
        self.assertTrue(stopped["unread"])
        self.assertTrue(next(r for r in self.host.request("list", {}) if r["id"] == b["id"])["failed"])

    def test_large_changes_are_summarized_within_limits(self):
        big = self.commit("big", ("x" * 60 + "\n") * 2000)
        small = self.git("rev-parse", "HEAD~1")
        sources, stats = studio.evidence(self.repo, [big, small])
        self.assertEqual(1, stats["summarized"])
        self.assertIn("변경 내용 생략", sources[0]["text"])
        self.assertIn("+third line", sources[1]["text"])
        self.assertLessEqual(len(sources[0]["text"]), studio.COMMIT_LIMIT)
        self.assertEqual(sources, studio.evidence(self.repo, [big, small])[0])

    def test_saved_edits_and_version_one_articles(self):
        a = self.run_all(self.generate(), FakeModel("pass"))
        edited = self.host.request("save", {"id": a["id"], "revision": a["revision"], "draft": "사람이 고침"})
        self.assertEqual(("edited", None), (edited["state"], edited["review"]))
        with self.assertRaisesRegex(ValueError, "다른 창"):
            self.host.request("save", {"id": a["id"], "revision": a["revision"], "draft": "오래된 입력"})
        old = {"id": "v1", "topic": "예전 글", "draft": "본문", "revision": 3, "job": None,
               "review": {"state": "reviewed", "findings": []}, "figures": [], "messages": []}
        self.host.db.execute("INSERT INTO article VALUES (?,?)", ("v1", json.dumps(old)))
        shown = self.host.request("open", {"id": "v1"})
        self.assertEqual((1, "예전 글", "reviewed"), (shown["version"], shown["title"], shown["state"]))
        self.assertIn({"id": "v1", "title": "예전 글", "state": "reviewed", "stage": None, "task": None,
                       "updated": None, "unread": False, "failed": False}, self.host.request("list", {}))
        self.host.request("delete", {"id": a["id"]})
        with self.assertRaisesRegex(ValueError, "찾을 수 없"):
            self.host.request("open", {"id": a["id"]})

    def test_disabled_extension_rejects_requests_and_stops_jobs(self):
        a = self.generate()
        self.host.request("configure", {"enabled": False})
        with self.assertRaisesRegex(ValueError, "먼저 켜"):
            self.host.request("list", {})
        self.host.request("configure", {"enabled": True})
        self.assertEqual({"more": False}, self.host.execute_job(a["id"], a["job"]["id"]))
        self.assertEqual("stopped", self.host.request("open", {"id": a["id"]})["state"])

    def test_manual_rewrite_keeps_the_previous_edition_and_does_not_rewrite_twice(self):
        a = self.run_all(self.generate(), FakeModel("revise", "revise"))
        self.assertEqual("findings", a["state"])
        running = self.host.request("generate", {"commits": [self.host.request("targets", {})["commits"][0]["sha"]]})
        with self.assertRaisesRegex(ValueError, "작업이 끝난 뒤"):
            self.host.request("revise", {"id": running["id"], "revision": running["revision"]})
        with self.assertRaisesRegex(ValueError, "다른 글을 만드는 중"):
            self.host.request("revise", {"id": a["id"], "revision": a["revision"]})
        self.host.request("cancel", {"id": running["id"]})
        job = self.host.request("revise", {"id": a["id"], "revision": a["revision"], "instruction": "더 짧게"})
        self.assertEqual(("running", "revise", "revise"), (job["state"], job["stage"], job["task"]))
        model = FakeModel("revise", text="# 짧은 글")
        b = self.run_all(job, model)
        self.assertEqual(["write", "review"], [kind for kind, _ in model.prompts])
        self.assertIn("더 짧게", model.prompts[0][1])
        self.assertEqual(("findings", "# 짧은 글", 2), (b["state"], b["draft"], b["edition"]))
        self.assertEqual("# 키 생성 경합 막기\n본문", b["previous"]["draft"])
        c = self.host.request("restore", {"id": a["id"], "revision": b["revision"]})
        self.assertEqual(("# 키 생성 경합 막기\n본문", "# 짧은 글"), (c["draft"], c["previous"]["draft"]))
        checked = self.run_all(self.host.request("review", {"id": a["id"], "revision": c["revision"]}), FakeModel("pass"))
        self.assertEqual(("reviewed", 3), (checked["state"], checked["edition"]))
        before = checked["previous"]
        job = self.host.request("revise", {"id": a["id"], "revision": checked["revision"], "instruction": "x"})
        failed = self.run_all(job, FakeModel(fail_on="write"))
        self.assertEqual(("reviewed", None, 3), (failed["state"], failed["stage"], failed["edition"]))
        self.assertEqual(("model_timeout", before), (failed["stopped"]["code"], failed["previous"]))
        job = self.host.request("revise", {"id": a["id"], "revision": failed["revision"], "instruction": "y"})
        self.assertIsNone(job["stopped"])
        cancelled = self.run_all(job, FakeModel(fail_on="review"))
        self.assertEqual(("edited", 4, "# 키 생성 경합 막기\n본문"), (cancelled["state"], cancelled["edition"], cancelled["draft"]))

    def test_saved_edit_is_a_new_edition(self):
        a = self.run_all(self.generate(), FakeModel("pass"))
        b = self.host.request("save", {"id": a["id"], "revision": a["revision"], "draft": "사람의 판"})
        self.assertEqual(("edited", "사람의 판", "reviewed"), (b["state"], b["draft"], b["previous"]["state"]))

    def test_export_writes_one_post_per_article_and_refuses_foreign_files(self):
        a = self.run_all(self.generate(), FakeModel("pass"))
        b = self.host.request("export", {"id": a["id"], "revision": a["revision"]})
        post = self.repo / b["exported"]["path"]
        self.assertRegex(b["exported"]["path"], r"^blog/\d{4}-\d{2}-\d{2}-key-race/post.md$")
        text = post.read_text(encoding="utf-8")
        self.assertTrue(text.startswith("---\ntitle: \"키 생성 경합 막기\"\n"))
        self.assertIn(f"akra_article: {a['id']}", text)
        self.assertTrue(text.endswith("본문\n"))
        again = self.host.request("export", {"id": a["id"], "revision": b["revision"]})
        self.assertEqual(b["exported"]["path"], again["exported"]["path"])
        post.write_text("다른 사람의 글", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "다른 파일"):
            self.host.request("export", {"id": a["id"], "revision": again["revision"]})
        post.unlink()
        post.parent.rmdir()
        outside = self.repo.parent / "outside"
        outside.mkdir()
        post.parent.symlink_to(outside, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "다른 파일"):
            self.host.request("export", {"id": a["id"], "revision": again["revision"]})
        self.assertEqual([], list(outside.iterdir()))

    def test_export_overwrites_only_figures_it_wrote(self):
        a = self.run_all(self.generate(), FakeModel("pass"))
        b = self.host.request("export", {"id": a["id"], "revision": a["revision"]})
        folder = (self.repo / b["exported"]["path"]).parent
        (folder / "fig1.png").write_bytes(b"user file")
        figure = {"id": "fig1", "path": "fig1.png", "alt": "그림", "caption": "그림"}
        b["figures"].append(figure)
        self.host.db.execute("UPDATE article SET value=? WHERE id=?", (json.dumps(b), a["id"]))
        with self.assertRaisesRegex(ValueError, "다른 파일"):
            self.host.request("export", {"id": a["id"], "revision": b["revision"]})
        self.assertEqual(b"user file", (folder / "fig1.png").read_bytes())

    def test_registering_a_figure_makes_a_new_edition(self):
        try:
            from io import BytesIO
            from PIL import Image
        except ImportError:
            self.skipTest("Pillow 가 없다")
        buffer = BytesIO()
        Image.new("RGB", (4, 4)).save(buffer, format="PNG")
        a = self.run_all(self.generate(), FakeModel("pass"))
        b = self.host.request("export", {"id": a["id"], "revision": a["revision"]})
        import base64
        c = self.host.request("figure", {"id": a["id"], "revision": b["revision"], "caption": "화면",
                                         "png": base64.b64encode(buffer.getvalue()).decode()})
        self.assertEqual((2, a["draft"]), (c["edition"], c["previous"]["draft"]))
        self.assertTrue(c["draft"].endswith("![화면](fig1.png)\n"))
        self.assertNotEqual(c["edition"], c["exported"]["edition"])

    def test_articles_from_slice_one_read_with_edition_defaults(self):
        a = self.run_all(self.generate(), FakeModel("pass"))
        stored = {k: v for k, v in a.items() if k not in ("edition", "previous", "exported", "task", "slug")}
        self.host.db.execute("UPDATE article SET value=? WHERE id=?", (json.dumps(stored), a["id"]))
        shown = self.host.request("open", {"id": a["id"]})
        self.assertEqual((1, None, None), (shown["edition"], shown["previous"], shown["exported"]))
        b = self.host.request("export", {"id": a["id"], "revision": shown["revision"]})
        self.assertEqual(b["edition"], b["exported"]["edition"])


if __name__ == "__main__":
    unittest.main()
