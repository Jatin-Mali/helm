"""Regression checks for the acceptance driver; no provider or HELM state required."""
import pathlib
import subprocess
import sys
import tempfile
import unittest


class AgentDriverTests(unittest.TestCase):
    def drive(self, source, *options):
        with tempfile.TemporaryDirectory() as work:
            return subprocess.run(
                [sys.executable, str(pathlib.Path(__file__).with_name("drive_agent.py")),
                 "--transcript", f"{work}/transcript", *options,
                 "--", sys.executable, "-c", source],
                capture_output=True, text=True, timeout=10,
            )

    def test_success_and_failure_exit_codes(self):
        self.assertEqual(self.drive("raise SystemExit(0)").returncode, 0)
        self.assertEqual(self.drive("raise SystemExit(7)").returncode, 7)

    def test_timeout_fails(self):
        self.assertEqual(self.drive("import time; time.sleep(5)", "--timeout", "0.1").returncode, 124)

    def test_exact_allow_pattern_matches_current_description(self):
        source = (
            "print('Approval required:\\nRun shell command:\\nuname -s\\nBROAD OS AUTHORITY: fixture'); "
            "answer=input('Approve this exact action once? [y/N] '); "
            "raise SystemExit(0 if answer == 'y' else 7)"
        )
        self.assertEqual(self.drive(source, "--allow", r"\ARun shell command:\nuname -s\nBROAD OS AUTHORITY:").returncode, 0)

    def test_prior_output_cannot_authorize_a_different_prompt(self):
        source = (
            "print('Run shell command:\\nuname -s\\nBROAD OS AUTHORITY: fixture'); "
            "print('Approval required:\\nRun shell command:\\ntouch marker\\nBROAD OS AUTHORITY: fixture'); "
            "answer=input('Approve this exact action once? [y/N] '); "
            "raise SystemExit(0 if answer == 'n' else 7)"
        )
        self.assertEqual(self.drive(source, "--allow", r"\ARun shell command:\nuname -s\nBROAD OS AUTHORITY:").returncode, 0)

    def test_demo_write_rule_accepts_relative_and_absolute_target_only(self):
        script = pathlib.Path(__file__).parents[2] / "scripts/verify-demo.sh"
        pattern = next(line.split("'")[1] for line in script.read_text().splitlines()
                       if "--allow" in line and "Write" in line)
        for path, expected in [("calc.py", "y"), ("/tmp/work/calc.py", "y"), ("other.py", "n")]:
            source = (
                f"print('Approval required:\\nWrite {path} (1 bytes)\\nNew content:\\nx'); "
                "answer=input('Approve this exact action once? [y/N] '); "
                f"raise SystemExit(0 if answer == {expected!r} else 7)"
            )
            self.assertEqual(self.drive(source, "--allow", pattern).returncode, 0, path)


if __name__ == "__main__":
    unittest.main()
