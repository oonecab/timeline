"""Run reproducible local acceptance checks without touching the user's DB."""

import json
from pathlib import Path
import subprocess
import sys
import time


def main() -> int:
    root = Path(__file__).resolve().parents[1]
    output = root / "target" / "acceptance"
    output.mkdir(parents=True, exist_ok=True)
    commands = [
        ["cargo", "fmt", "--all", "--", "--check"],
        ["cargo", "clippy", "--all-targets", "--locked", "--", "-D", "warnings"],
        ["cargo", "test", "--all-targets", "--locked"],
        ["cargo", "build", "--release", "--bins", "--locked"],
    ]
    results = []
    with (output / "commands.log").open("w", encoding="utf-8") as log:
        for command in commands:
            title = "$ " + " ".join(command)
            print(title, flush=True)
            log.write(title + "\n")
            started = time.monotonic()
            process = subprocess.Popen(
                command,
                cwd=root,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                encoding="utf-8",
                errors="replace",
            )
            for line in process.stdout:
                print(line, end="", flush=True)
                log.write(line)
            code = process.wait()
            result = {
                "command": command,
                "exit_code": code,
                "seconds": round(time.monotonic() - started, 3),
            }
            results.append(result)
            log.write(f"exit_code={code}\n\n")
            log.flush()
            if code:
                break
    (output / "results.json").write_text(
        json.dumps(results, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    return 0 if len(results) == len(commands) and all(r["exit_code"] == 0 for r in results) else 1


if __name__ == "__main__":
    sys.exit(main())
