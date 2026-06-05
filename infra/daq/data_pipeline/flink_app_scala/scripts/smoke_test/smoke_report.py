"""Report formatting for smoke tests."""

import json
import os
from dataclasses import dataclass, field, asdict
from datetime import datetime, timezone
from typing import Optional


@dataclass
class SmokeScenarioResult:
    name: str
    status: str  # "PASS" or "FAIL"
    duration_s: float
    error: Optional[str] = None
    expected: Optional[str] = None
    actual: Optional[str] = None


@dataclass
class SmokeReport:
    layer: str = "smoke"
    timestamp: str = ""
    duration_s: float = 0.0
    total: int = 0
    passed: int = 0
    failed: int = 0
    scenarios: list = field(default_factory=list)

    def add(self, result: SmokeScenarioResult):
        self.scenarios.append(result)
        self.total += 1
        if result.status == "PASS":
            self.passed += 1
        else:
            self.failed += 1
        self.duration_s += result.duration_s

    def print_console(self):
        print("=== Scenario Test Report ===")
        print(
            f"Layer: {self.layer} | {self.total} scenarios | "
            f"{self.passed} passed | {self.failed} failed | "
            f"{self.duration_s:.1f}s"
        )
        print()
        for s in self.scenarios:
            tag = "[PASS]" if s.status == "PASS" else "[FAIL]"
            print(f"  {tag:<6} {s.name:<50} ({s.duration_s:.1f}s)")

        failures = [s for s in self.scenarios if s.status == "FAIL"]
        if failures:
            print()
            print("--- FAILURE DETAILS ---")
            print()
            for f in failures:
                print(f"{f.name}:")
                if f.expected:
                    print(f"  Expected: {f.expected}")
                if f.actual:
                    print(f"  Actual:   {f.actual}")
                if f.error:
                    print(f"  Error:    {f.error}")
                print()

    def write_json(self, output_dir="target/test-reports"):
        os.makedirs(output_dir, exist_ok=True)
        ts = datetime.now(timezone.utc).isoformat().replace(":", "-")
        path = os.path.join(output_dir, f"scenario-smoke-{ts}.json")
        with open(path, "w") as f:
            json.dump(asdict(self), f, indent=2, default=str)
        return path
