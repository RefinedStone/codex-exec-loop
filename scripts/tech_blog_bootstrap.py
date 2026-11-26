"""Run only worker sources bundled in the installed kernel, independently of checkout contents."""
import json
import os
import sys
import tempfile
from pathlib import Path


def main():
    message = json.load(sys.stdin.buffer)
    for name in list(os.environ):
        if name.upper().startswith("GIT_"):
            del os.environ[name]
    with tempfile.TemporaryDirectory(prefix="akra-blog-host-") as directory:
        root = Path(directory)
        for name in ("tech_blog.py", "tech_blog_studio.py", "tech_blog_review.schema.json", "tech_blog_write.schema.json",
                     "akra-tech-blog.SKILL.md"):
            (root/name).write_text(message["scripts"][name], encoding="utf-8")
        sys.path.insert(0, directory)
        import tech_blog
        tech_blog.GIT_PROGRAM = message["git"]
        tech_blog.MODEL_COMMAND = message["model_command"] or []
        from tech_blog_studio import Studio
        studio = Studio(Path(message["runtime"]), Path(message["workspace"]), message["owner"])
        data = message["input"]
        try:
            if message["action"] == "execute":
                return studio.execute_job(data["id"], data["job"])
            if message["action"] == "failed":
                return studio.fail_job(data["id"], data["job"])
            return studio.request(message["action"], data)
        finally:
            studio.db.close()


try:
    print(json.dumps({"value": main()}, ensure_ascii=True))
except Exception as error:
    print(json.dumps({"error": str(error)}, ensure_ascii=True))
