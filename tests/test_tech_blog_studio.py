import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import tech_blog_studio as studio


class StudioInteractionTest(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.runtime = Path(temp.name)
        self.repo = Path(__file__).resolve().parents[1]
        self.host = studio.Studio(self.runtime, self.repo, "first-kernel")
        self.addCleanup(self.host.db.close)

    def create(self):
        self.host.request("configure", {"enabled": True})
        return self.host.request("create", {"topic": "Git 근거로 글 쓰기", "style": "PROBLEM_SOLVING",
                                 "mode": "PROJECT_RETROSPECTIVE", "paths": ["README.md"]})

    def apply(self, a, action, **data):
        return self.host.request(action, dict(id=a["id"], revision=a["revision"], **data))

    def read(self, a):
        return self.host.request("open", {"id": a["id"]})

    def complete(self, a):
        self.host.execute_job(a["id"], a["job"]["id"])
        return self.read(a)

    def test_collaborative_edit_review_and_resume(self):
        a = self.create()
        job = self.apply(a, "write", model="fixture", instruction="관찰만 설명")
        proposal = {"draft": "# 첫 제안", "reply": "원문과 대조했습니다.",
                    "claims": [{"id": "c1", "text": "검증 대상", "sources": ["s1"]}]}
        with patch.object(studio.blog, "run_codex_json", return_value=proposal) as model:
            result = self.complete(job)
            self.assertIn("README.md", model.call_args.args[1].decode())
        self.assertEqual("", result["draft"])
        self.assertEqual(proposal, result["proposal"])
        a = self.apply(result, "accept")
        a = self.apply(a, "save", draft=a["draft"]+"\n사람이 수정")
        with self.assertRaisesRegex(ValueError, "다른 창"):
            self.apply(result, "save", draft="오래된 입력")
        result = self.complete(self.apply(a, "review", model="fixture"))
        self.assertEqual("needs_evidence", result["review"]["state"])
        job = self.apply(result, "write", model="fixture", instruction="검수 의견을 반영해 줘")
        with patch.object(studio.blog, "run_codex_json", return_value=proposal) as model:
            result = self.complete(job)
            prompt = model.call_args.args[1].decode()
            for expected in ("사람이 수정", "figure_required", "검수 의견을 반영해 줘"):
                self.assertIn(expected, prompt)
        second = studio.Studio(self.runtime, self.repo, "next-kernel")
        self.addCleanup(second.db.close)
        self.assertEqual(result, second.request("open", {"id": a["id"]}))

    def test_disabled_and_restarted_jobs_cannot_change_current_draft(self):
        with self.assertRaisesRegex(ValueError, "먼저 켜"):
            self.host.request("list", {})
        a = self.create()
        job = self.apply(a, "write", model="fixture")
        self.host.request("configure", {"enabled": False})
        with patch.object(studio, "write_proposal", side_effect=AssertionError("disabled job ran")):
            self.host.execute_job(a["id"], job["job"]["id"])
        self.host.request("configure", {"enabled": True})
        a = self.read(a)
        self.assertIsNone(a["job"])
        job = self.apply(a, "write", model="fixture")
        second = studio.Studio(self.runtime, self.repo, "next-kernel")
        self.addCleanup(second.db.close)
        recovered = second.request("open", {"id": a["id"]})
        self.assertIsNone(recovered["job"])
        self.host.execute_job(a["id"], job["job"]["id"])
        self.assertEqual(recovered, second.read(a["id"]))

    def test_topic_only_create_selects_pinned_evidence(self):
        self.host.request("configure", {"enabled":True})
        a = self.host.request("create", {"topic":"Desktop", "style":"TECHNICAL_DEEP_DIVE", "mode":"TECH_EXPLORATION"})
        self.assertTrue(a["sources"])
        self.assertEqual(40,len(a["commit"]))
        self.assertTrue(studio.blog.collect_excerpt(self.repo,a["commit"],a["sources"][0])["text"])
