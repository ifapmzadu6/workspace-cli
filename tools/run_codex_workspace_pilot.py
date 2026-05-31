#!/usr/bin/env python3
"""Run a small Codex pilot comparing shell-only and workspace-cli-assisted work."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, NamedTuple


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_OUTPUT_DIR = ROOT / "target" / "codex-workspace-pilot"
DEFAULT_TASK = "discounted_tax_bug"
TEST_COMMAND = "PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tests"


class Condition(NamedTuple):
    name: str
    prompt: str


class TaskSpec(NamedTuple):
    name: str
    prompt: str
    workspace_extra: str
    create_repo: Callable[[Path, Path], None]
    expected_changed_files: tuple[str, ...]


def write_text(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def run_command(
    command: list[str],
    *,
    cwd: Path,
    input_text: str | None = None,
    timeout_seconds: int = 120,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        cwd=cwd,
        input=input_text,
        text=True,
        capture_output=True,
        timeout=timeout_seconds,
        check=False,
    )


def resolve_workspace_binary(path: Path | None) -> Path:
    if path is not None:
        candidate = path if path.is_absolute() else (Path.cwd() / path)
        return candidate.resolve()
    local = ROOT / "target" / "debug" / "workspace"
    if local.is_file():
        return local
    found = shutil.which("workspace")
    if found:
        return Path(found).resolve()
    raise FileNotFoundError(
        "workspace binary not found; run `cargo build` or pass --workspace-bin"
    )


def install_workspace_binary(repo: Path, workspace_binary: Path) -> None:
    bin_dir = repo / "bin"
    bin_dir.mkdir()
    copied_workspace = bin_dir / "workspace"
    shutil.copy2(workspace_binary, copied_workspace)
    copied_workspace.chmod(copied_workspace.stat().st_mode | 0o111)


def init_repo(repo: Path) -> None:
    run_command(["git", "init", "-q"], cwd=repo)
    run_command(["git", "config", "user.email", "pilot@example.test"], cwd=repo)
    run_command(["git", "config", "user.name", "Codex Pilot"], cwd=repo)
    info_exclude = repo / ".git" / "info" / "exclude"
    with info_exclude.open("a", encoding="utf-8") as exclude:
        exclude.write("\n/bin/workspace\n")


def commit_all(repo: Path, message: str) -> None:
    run_command(["git", "add", "."], cwd=repo)
    result = run_command(["git", "commit", "-q", "-m", message], cwd=repo)
    if result.returncode != 0:
        raise RuntimeError(result.stderr or result.stdout or "git commit failed")


def create_checkout_fixture_repo(repo: Path, workspace_binary: Path) -> None:
    repo.mkdir(parents=True, exist_ok=True)
    write_text(
        repo / "src" / "checkout.py",
        """\
def calculate_order_total(items, discount_rate, tax_rate, shipping):
    subtotal = sum(item["price"] * item["quantity"] for item in items)
    discount = round(subtotal * discount_rate, 2)
    discounted_subtotal = subtotal - discount
    tax = round(subtotal * tax_rate, 2)
    return round(discounted_subtotal + tax + shipping, 2)
""",
    )
    write_text(repo / "src" / "__init__.py", "")
    write_text(
        repo / "tests" / "test_checkout.py",
        """\
import unittest

from src.checkout import calculate_order_total


class CheckoutTests(unittest.TestCase):
    def test_taxes_discounted_subtotal(self):
        total = calculate_order_total(
            [{"price": 50.0, "quantity": 2}],
            discount_rate=0.20,
            tax_rate=0.10,
            shipping=5.0,
        )
        self.assertEqual(total, 93.0)

    def test_no_discount_path_is_unchanged(self):
        total = calculate_order_total(
            [{"price": 12.5, "quantity": 4}],
            discount_rate=0.0,
            tax_rate=0.10,
            shipping=0.0,
        )
        self.assertEqual(total, 55.0)


if __name__ == "__main__":
    unittest.main()
""",
    )
    write_text(
        repo / "docs" / "checkout.md",
        """\
# Checkout Rules

Discounts apply before tax. Shipping is added after tax.
""",
    )
    write_text(
        repo / "README.md",
        f"""\
# Checkout Fixture

Run tests with:

```sh
{TEST_COMMAND}
```
""",
    )
    install_workspace_binary(repo, workspace_binary)
    init_repo(repo)
    commit_all(repo, "Initial checkout fixture")


def write_policy_files(
    repo: Path,
    *,
    max_discount_cents: int,
    test_expected_cents: int,
) -> None:
    write_text(
        repo / "src" / "discounts.py",
        """\
import json
from pathlib import Path


POLICY_PATH = Path(__file__).resolve().parent.parent / "config" / "discount_policy.json"


def load_policy():
    return json.loads(POLICY_PATH.read_text(encoding="utf-8"))


def premium_discount_cents(order_total_cents):
    policy = load_policy()["premium"]
    if order_total_cents < policy["minimum_order_cents"]:
        return 0
    raw_discount = round(order_total_cents * policy["rate_bps"] / 10_000)
    return min(raw_discount, policy["max_discount_cents"])
""",
    )
    write_text(repo / "src" / "__init__.py", "")
    write_text(
        repo / "config" / "discount_policy.json",
        json.dumps(
            {
                "premium": {
                    "minimum_order_cents": 5_000,
                    "rate_bps": 2_500,
                    "max_discount_cents": max_discount_cents,
                }
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
    )
    dollars = f"${max_discount_cents / 100:.2f}"
    write_text(
        repo / "docs" / "discount_policy.md",
        f"""\
# Discount Policy

Premium customers receive 25% off qualifying orders, capped at {dollars}.
""",
    )
    expected_dollars = f"${test_expected_cents / 100:.2f}"
    write_text(
        repo / "tests" / "test_discounts.py",
        f"""\
import json
import unittest
from pathlib import Path

from src.discounts import premium_discount_cents


ROOT = Path(__file__).resolve().parent.parent


class DiscountPolicyTests(unittest.TestCase):
    def test_premium_discount_cap_matches_current_policy(self):
        self.assertEqual(premium_discount_cents(20_000), {test_expected_cents})

    def test_policy_config_records_current_cap(self):
        policy = json.loads((ROOT / "config" / "discount_policy.json").read_text())
        self.assertEqual(policy["premium"]["max_discount_cents"], {test_expected_cents})

    def test_docs_record_current_cap(self):
        docs = (ROOT / "docs" / "discount_policy.md").read_text()
        self.assertIn("{expected_dollars}", docs)


if __name__ == "__main__":
    unittest.main()
""",
    )
    write_text(
        repo / "README.md",
        f"""\
# Discount Policy Fixture

Run tests with:

```sh
{TEST_COMMAND}
```
""",
    )


def create_policy_fixture_repo(repo: Path, workspace_binary: Path) -> None:
    repo.mkdir(parents=True, exist_ok=True)
    install_workspace_binary(repo, workspace_binary)
    init_repo(repo)
    write_policy_files(repo, max_discount_cents=1_500, test_expected_cents=1_500)
    commit_all(repo, "Initial premium discount policy")
    write_policy_files(repo, max_discount_cents=2_000, test_expected_cents=2_000)
    commit_all(repo, "Raise premium discount cap to 20 dollars")
    write_policy_files(repo, max_discount_cents=2_000, test_expected_cents=2_500)
    commit_all(repo, "Add tests for 25 dollar premium cap")


def write_late_fee_files(
    repo: Path,
    *,
    cap_cents: int,
    test_expected_cap_cents: int,
) -> None:
    cap_literal = "1_000" if cap_cents == 1_000 else str(cap_cents)
    write_text(
        repo / "src" / "billing.py",
        f"""\
LATE_FEE_RATE_CENTS = 150
LATE_FEE_CAP_CENTS = {cap_literal}


def late_fee_cents(days_late):
    if days_late <= 0:
        return 0
    return min(days_late * LATE_FEE_RATE_CENTS, LATE_FEE_CAP_CENTS)
""",
    )
    write_text(repo / "src" / "__init__.py", "")
    dollars = f"${cap_cents / 100:.2f}"
    write_text(
        repo / "docs" / "billing.md",
        f"""\
# Billing Rules

Late fees are 150 cents per day and capped at {dollars}.
""",
    )
    expected_dollars = f"${test_expected_cap_cents / 100:.2f}"
    write_text(
        repo / "tests" / "test_billing.py",
        f"""\
import unittest
from pathlib import Path

from src.billing import LATE_FEE_CAP_CENTS, late_fee_cents


ROOT = Path(__file__).resolve().parent.parent


class BillingTests(unittest.TestCase):
    def test_short_late_fee_uses_daily_rate(self):
        self.assertEqual(late_fee_cents(3), 450)

    def test_late_fee_cap_matches_current_policy(self):
        self.assertEqual(late_fee_cents(20), {test_expected_cap_cents})
        self.assertEqual(LATE_FEE_CAP_CENTS, {test_expected_cap_cents})

    def test_docs_record_current_late_fee_cap(self):
        docs = (ROOT / "docs" / "billing.md").read_text()
        self.assertIn("{expected_dollars}", docs)


if __name__ == "__main__":
    unittest.main()
""",
    )
    write_text(
        repo / "README.md",
        f"""\
# Billing Fixture

Run tests with:

```sh
{TEST_COMMAND}
```
""",
    )


def write_bad_late_fee_patch(repo: Path) -> None:
    write_text(
        repo / "docs" / "proposed_late_fee_fix.patch",
        "\n".join(
            [
                "diff --git a/src/billing.py b/src/billing.py",
                "--- a/src/billing.py",
                "+++ b/src/billing.py",
                "@@ -1,5 +1,5 @@",
                "-LATE_FEE_RATE_CENTS = 150",
                "-LATE_FEE_CAP_CENTS = 1_000",
                "+LATE_FEE_RATE_CENTS = 200",
                "+LATE_FEE_CAP_CENTS = 1500",
                " ",
                " ",
                " def late_fee_cents(days_late):",
                "diff --git a/docs/billing.md b/docs/billing.md",
                "--- a/docs/billing.md",
                "+++ b/docs/billing.md",
                "@@ -1,3 +1,3 @@",
                " # Billing Rules",
                " ",
                "-Late fees are 150 cents per day and capped at $10.00.",
                "+Late fees are 200 cents per day and capped at $15.00.",
                "",
            ]
        ),
    )


def create_rollback_fixture_repo(repo: Path, workspace_binary: Path) -> None:
    repo.mkdir(parents=True, exist_ok=True)
    install_workspace_binary(repo, workspace_binary)
    init_repo(repo)
    write_late_fee_files(repo, cap_cents=1_000, test_expected_cap_cents=1_000)
    commit_all(repo, "Initial late fee policy")
    write_late_fee_files(repo, cap_cents=1_000, test_expected_cap_cents=1_500)
    write_bad_late_fee_patch(repo)
    commit_all(repo, "Add late fee cap tests and proposed patch")


def write_invoice_tax_files(
    repo: Path,
    *,
    vat_bps: int,
    test_expected_bps: int,
) -> None:
    write_text(
        repo / "src" / "invoice.py",
        """\
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
TAX_POLICY_PATH = ROOT / "config" / "tax_regions.json"
LABELS_PATH = ROOT / "config" / "invoice_labels.json"


def load_tax_policy():
    return json.loads(TAX_POLICY_PATH.read_text(encoding="utf-8"))


def load_invoice_labels():
    return json.loads(LABELS_PATH.read_text(encoding="utf-8"))


def digital_vat_cents(amount_cents):
    rate_bps = load_tax_policy()["eu"]["digital_vat_bps"]
    return round(amount_cents * rate_bps / 10_000)


def digital_vat_label():
    return load_invoice_labels()["tax_lines"]["eu_digital_vat"]
""",
    )
    write_text(repo / "src" / "__init__.py", "")
    write_text(
        repo / "config" / "tax_regions.json",
        json.dumps(
            {
                "eu": {
                    "digital_vat_bps": vat_bps,
                    "reverse_charge_enabled": False,
                },
                "us": {"digital_vat_bps": 0},
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
    )
    vat_percent = vat_bps / 100
    write_text(
        repo / "config" / "invoice_labels.json",
        json.dumps(
            {"tax_lines": {"eu_digital_vat": f"EU digital VAT {vat_percent:.0f}%"}},
            indent=2,
            sort_keys=True,
        )
        + "\n",
    )
    expected_percent = test_expected_bps / 100
    write_text(
        repo / "docs" / "tax_policy.md",
        f"""\
# Tax Policy

EU digital invoices use a {vat_percent:.0f}% VAT rate.
""",
    )
    write_text(
        repo / "docs" / "invoice_templates.md",
        f"""\
# Invoice Templates

The EU digital VAT line is rendered as "EU digital VAT {vat_percent:.0f}%".
""",
    )
    write_text(
        repo / "tests" / "test_invoice_pipeline.py",
        f"""\
import json
import unittest
from pathlib import Path

from src.invoice import digital_vat_cents, digital_vat_label


ROOT = Path(__file__).resolve().parent.parent


class InvoicePipelineTests(unittest.TestCase):
    def test_eu_digital_vat_rate_matches_current_policy(self):
        policy = json.loads((ROOT / "config" / "tax_regions.json").read_text())
        self.assertEqual(policy["eu"]["digital_vat_bps"], {test_expected_bps})
        self.assertEqual(digital_vat_cents(10_000), {test_expected_bps})

    def test_invoice_label_matches_current_policy(self):
        labels = json.loads((ROOT / "config" / "invoice_labels.json").read_text())
        self.assertEqual(labels["tax_lines"]["eu_digital_vat"], "EU digital VAT {expected_percent:.0f}%")
        self.assertEqual(digital_vat_label(), "EU digital VAT {expected_percent:.0f}%")

    def test_docs_match_current_policy(self):
        tax_docs = (ROOT / "docs" / "tax_policy.md").read_text()
        template_docs = (ROOT / "docs" / "invoice_templates.md").read_text()
        self.assertIn("{expected_percent:.0f}% VAT", tax_docs)
        self.assertIn("EU digital VAT {expected_percent:.0f}%", template_docs)


if __name__ == "__main__":
    unittest.main()
""",
    )
    write_text(
        repo / "README.md",
        f"""\
# Invoice Fixture

Run tests with:

```sh
{TEST_COMMAND}
```
""",
    )


def write_invoice_decoy_files(repo: Path) -> None:
    write_text(
        repo / "src" / "shipping.py",
        """\
def shipping_zone(country):
    return "domestic" if country == "US" else "international"
""",
    )
    write_text(
        repo / "src" / "promotions.py",
        """\
def promotion_label(code):
    return code.upper()
""",
    )
    write_text(
        repo / "config" / "shipping_zones.json",
        json.dumps({"US": "domestic", "DE": "international"}, indent=2) + "\n",
    )
    write_text(
        repo / "config" / "promotion_rules.json",
        json.dumps({"welcome": {"percent_off": 10}}, indent=2) + "\n",
    )
    write_text(
        repo / "docs" / "shipping_policy.md",
        "# Shipping Policy\n\nInternational shipping uses zone-based pricing.\n",
    )
    write_text(
        repo / "docs" / "promotion_policy.md",
        "# Promotion Policy\n\nPromotion labels are uppercase codes.\n",
    )


def create_invoice_tax_fixture_repo(repo: Path, workspace_binary: Path) -> None:
    repo.mkdir(parents=True, exist_ok=True)
    install_workspace_binary(repo, workspace_binary)
    init_repo(repo)
    write_invoice_tax_files(repo, vat_bps=1_900, test_expected_bps=1_900)
    write_invoice_decoy_files(repo)
    commit_all(repo, "Initial invoice tax policy")
    write_invoice_tax_files(repo, vat_bps=2_100, test_expected_bps=2_100)
    write_invoice_decoy_files(repo)
    commit_all(repo, "Raise EU digital VAT to 21 percent")
    write_invoice_tax_files(repo, vat_bps=2_100, test_expected_bps=2_200)
    write_invoice_decoy_files(repo)
    commit_all(repo, "Add tests for 22 percent EU digital VAT")


def write_subscription_plan_files(
    repo: Path,
    *,
    seat_limit: int,
    audit_retention_days: int,
    advanced_audit_exports: bool,
    display_name: str,
    monthly_price_cents: int,
    test_expected_seat_limit: int,
    test_expected_audit_retention_days: int,
    test_expected_advanced_audit_exports: bool,
    test_expected_display_name: str,
    test_expected_monthly_price_cents: int,
) -> None:
    write_text(
        repo / "src" / "plans.py",
        """\
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
LIMITS_PATH = ROOT / "config" / "plan_limits.json"
FEATURES_PATH = ROOT / "config" / "feature_flags.json"
CATALOG_PATH = ROOT / "config" / "billing_catalog.json"


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def plan_limits(plan):
    return read_json(LIMITS_PATH)["plans"][plan]


def plan_features(plan):
    return read_json(FEATURES_PATH)["plans"][plan]


def catalog_entry(plan):
    return read_json(CATALOG_PATH)["plans"][plan]


def plan_summary(plan):
    limits = plan_limits(plan)
    features = plan_features(plan)
    catalog = catalog_entry(plan)
    return {
        "display_name": catalog["display_name"],
        "monthly_price_cents": catalog["monthly_price_cents"],
        "seat_limit": limits["seat_limit"],
        "audit_retention_days": limits["audit_retention_days"],
        "advanced_audit_exports": features["advanced_audit_exports"],
    }


def welcome_email(plan):
    return (ROOT / "templates" / f"{plan}_welcome_email.txt").read_text(
        encoding="utf-8"
    )
""",
    )
    write_text(repo / "src" / "__init__.py", "")
    write_text(
        repo / "config" / "plan_limits.json",
        json.dumps(
            {
                "plans": {
                    "starter": {
                        "seat_limit": 5,
                        "audit_retention_days": 30,
                    },
                    "growth": {
                        "seat_limit": 50,
                        "audit_retention_days": 180,
                    },
                    "enterprise": {
                        "seat_limit": seat_limit,
                        "audit_retention_days": audit_retention_days,
                    },
                }
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
    )
    write_text(
        repo / "config" / "feature_flags.json",
        json.dumps(
            {
                "plans": {
                    "starter": {"advanced_audit_exports": False, "sso": False},
                    "growth": {"advanced_audit_exports": False, "sso": True},
                    "enterprise": {
                        "advanced_audit_exports": advanced_audit_exports,
                        "sso": seat_limit >= 250,
                    },
                }
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
    )
    write_text(
        repo / "config" / "billing_catalog.json",
        json.dumps(
            {
                "plans": {
                    "starter": {
                        "display_name": "Starter",
                        "monthly_price_cents": 1900,
                    },
                    "growth": {
                        "display_name": "Growth",
                        "monthly_price_cents": 9900,
                    },
                    "enterprise": {
                        "display_name": display_name,
                        "monthly_price_cents": monthly_price_cents,
                    },
                }
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
    )
    price_dollars = monthly_price_cents // 100
    advanced_text = (
        "includes advanced audit exports"
        if advanced_audit_exports
        else "does not include advanced audit exports"
    )
    write_text(
        repo / "docs" / "plans.md",
        f"""\
# Plans

{display_name} includes {seat_limit} seats, {audit_retention_days} days of audit retention, and {advanced_text}.
""",
    )
    write_text(
        repo / "docs" / "api_contract.md",
        f"""\
# API Contract

The enterprise plan response exposes display_name="{display_name}", seat_limit={seat_limit}, audit_retention_days={audit_retention_days}, and monthly_price_cents={monthly_price_cents}.
""",
    )
    write_text(
        repo / "templates" / "enterprise_welcome_email.txt",
        f"""\
Welcome to {display_name}.

Your plan supports {seat_limit} seats and {audit_retention_days} days of audit history for ${price_dollars}/month.
""",
    )
    expected_advanced = str(test_expected_advanced_audit_exports)
    expected_price_dollars = test_expected_monthly_price_cents // 100
    write_text(
        repo / "tests" / "test_plan_rollout.py",
        f"""\
import json
import unittest
from pathlib import Path

from src.plans import plan_summary, welcome_email


ROOT = Path(__file__).resolve().parent.parent


class PlanRolloutTests(unittest.TestCase):
    def test_enterprise_plus_runtime_summary(self):
        summary = plan_summary("enterprise")
        self.assertEqual(summary["display_name"], "{test_expected_display_name}")
        self.assertEqual(summary["seat_limit"], {test_expected_seat_limit})
        self.assertEqual(summary["audit_retention_days"], {test_expected_audit_retention_days})
        self.assertEqual(
            summary["advanced_audit_exports"],
            {expected_advanced},
        )
        self.assertEqual(
            summary["monthly_price_cents"],
            {test_expected_monthly_price_cents},
        )

    def test_enterprise_plus_configs_are_synchronized(self):
        limits = json.loads((ROOT / "config" / "plan_limits.json").read_text())
        features = json.loads((ROOT / "config" / "feature_flags.json").read_text())
        catalog = json.loads((ROOT / "config" / "billing_catalog.json").read_text())
        self.assertEqual(
            limits["plans"]["enterprise"]["seat_limit"],
            {test_expected_seat_limit},
        )
        self.assertEqual(
            limits["plans"]["enterprise"]["audit_retention_days"],
            {test_expected_audit_retention_days},
        )
        self.assertEqual(
            features["plans"]["enterprise"]["advanced_audit_exports"],
            {expected_advanced},
        )
        self.assertEqual(
            catalog["plans"]["enterprise"]["display_name"],
            "{test_expected_display_name}",
        )

    def test_enterprise_plus_docs_and_templates_are_synchronized(self):
        plan_docs = (ROOT / "docs" / "plans.md").read_text()
        api_docs = (ROOT / "docs" / "api_contract.md").read_text()
        email = welcome_email("enterprise")
        for text in [plan_docs, api_docs, email]:
            self.assertIn("{test_expected_display_name}", text)
            self.assertIn("{test_expected_seat_limit}", text)
            self.assertIn("{test_expected_audit_retention_days}", text)
        self.assertIn("${expected_price_dollars}/month", email)


if __name__ == "__main__":
    unittest.main()
""",
    )
    write_text(
        repo / "README.md",
        f"""\
# Subscription Plan Fixture

Run tests with:

```sh
{TEST_COMMAND}
```
""",
    )


def write_subscription_decoy_files(repo: Path) -> None:
    write_text(
        repo / "src" / "support.py",
        """\
def support_tier(plan):
    return "standard" if plan != "enterprise" else "dedicated"
""",
    )
    write_text(
        repo / "config" / "support_tiers.json",
        json.dumps({"enterprise": "dedicated", "growth": "standard"}, indent=2)
        + "\n",
    )
    write_text(
        repo / "config" / "trial_limits.json",
        json.dumps({"starter_trial_days": 14, "growth_trial_days": 30}, indent=2)
        + "\n",
    )
    write_text(
        repo / "docs" / "support_policy.md",
        "# Support Policy\n\nEnterprise accounts receive dedicated support.\n",
    )
    write_text(
        repo / "docs" / "trial_policy.md",
        "# Trial Policy\n\nStarter trials last 14 days.\n",
    )
    write_text(
        repo / "templates" / "trial_welcome_email.txt",
        "Welcome to your trial.\n",
    )


def create_subscription_rollout_fixture_repo(
    repo: Path,
    workspace_binary: Path,
) -> None:
    repo.mkdir(parents=True, exist_ok=True)
    install_workspace_binary(repo, workspace_binary)
    init_repo(repo)
    write_subscription_plan_files(
        repo,
        seat_limit=100,
        audit_retention_days=365,
        advanced_audit_exports=False,
        display_name="Enterprise",
        monthly_price_cents=19900,
        test_expected_seat_limit=100,
        test_expected_audit_retention_days=365,
        test_expected_advanced_audit_exports=False,
        test_expected_display_name="Enterprise",
        test_expected_monthly_price_cents=19900,
    )
    write_subscription_decoy_files(repo)
    commit_all(repo, "Initial enterprise plan")
    write_subscription_plan_files(
        repo,
        seat_limit=250,
        audit_retention_days=365,
        advanced_audit_exports=False,
        display_name="Enterprise",
        monthly_price_cents=21900,
        test_expected_seat_limit=250,
        test_expected_audit_retention_days=365,
        test_expected_advanced_audit_exports=False,
        test_expected_display_name="Enterprise",
        test_expected_monthly_price_cents=21900,
    )
    write_subscription_decoy_files(repo)
    commit_all(repo, "Raise enterprise plan capacity")
    write_subscription_plan_files(
        repo,
        seat_limit=250,
        audit_retention_days=365,
        advanced_audit_exports=False,
        display_name="Enterprise",
        monthly_price_cents=21900,
        test_expected_seat_limit=500,
        test_expected_audit_retention_days=730,
        test_expected_advanced_audit_exports=True,
        test_expected_display_name="Enterprise Plus",
        test_expected_monthly_price_cents=24900,
    )
    write_subscription_decoy_files(repo)
    commit_all(repo, "Add tests for Enterprise Plus rollout")


def create_fixture_repo(repo: Path, workspace_binary: Path) -> None:
    create_checkout_fixture_repo(repo, workspace_binary)


def task_specs() -> dict[str, TaskSpec]:
    checkout_prompt = (
        "The repository has a failing checkout test. Fix the production bug with "
        f"the smallest reasonable change, run `{TEST_COMMAND}`, and leave the "
        "working tree with the fix applied. Do not edit tests."
    )
    policy_prompt = (
        "The repository has failing discount policy tests. The premium discount "
        "cap has changed to $25.00. Synchronize the production policy and "
        f"documentation, run `{TEST_COMMAND}`, and leave the working tree with "
        "the fix applied. Do not edit tests."
    )
    rollback_prompt = (
        "The repository has failing billing tests. The late-fee cap has changed "
        "to $15.00, but the daily late-fee rate must stay 150 cents. First "
        "apply `docs/proposed_late_fee_fix.patch`, run the tests, and inspect "
        "the failure. If that proposed patch fails, revert only that attempted "
        "patch before implementing the correct production and documentation "
        f"fix. Run `{TEST_COMMAND}`, leave the working tree with the correct "
        "fix applied, and do not edit tests."
    )
    invoice_tax_prompt = (
        "The repository has failing invoice pipeline tests. EU digital VAT has "
        "changed to 22%. Synchronize the production tax configuration, invoice "
        f"label configuration, and documentation, run `{TEST_COMMAND}`, and "
        "leave the working tree with the fix applied. Do not edit tests."
    )
    subscription_rollout_prompt = (
        "The repository has failing subscription plan rollout tests. The "
        "enterprise plan is launching as Enterprise Plus with 500 seats, 730 "
        "days of audit retention, advanced audit exports enabled, and a $249 "
        f"monthly price. Synchronize the runtime configs, documentation, and "
        f"welcome template, run `{TEST_COMMAND}`, and leave the working tree "
        "with the fix applied. Do not edit tests."
    )
    return {
        "discounted_tax_bug": TaskSpec(
            name="discounted_tax_bug",
            prompt=checkout_prompt,
            workspace_extra=(
                "Use `./bin/workspace` for workspace observation and "
                "verification whenever possible. Start with "
                "`./bin/workspace status --json`, use `read`, `search`, `diff`, "
                f"and run tests with `./bin/workspace run \"{TEST_COMMAND}\" "
                "--json`. If you edit files, prefer creating a patch and "
                "applying it with `./bin/workspace patch`."
            ),
            create_repo=create_checkout_fixture_repo,
            expected_changed_files=("src/checkout.py",),
        ),
        "policy_threshold_sync": TaskSpec(
            name="policy_threshold_sync",
            prompt=policy_prompt,
            workspace_extra=(
                "Use `./bin/workspace` for workspace observation and "
                "verification whenever possible. The command syntax below is "
                "complete; do not spend time running `workspace --help`, "
                "command-specific `--help`, or inspecting `.workspace` metadata. "
                "Do not run `workspace status` or `workspace index cochange` "
                "before the related-file query; `--ensure-index` creates or "
                "refreshes the co-change index in the same command. Start with "
                "`./bin/workspace related tests/test_discounts.py --by cochange "
                "--ensure-index --max-commits 1000 --rank hybrid --max-results 2 "
                "--include-content --max-content-files 2 --json` to find and "
                "read the policy and documentation files that usually change "
                "with this test. Use returned `data.included_content` instead "
                "of separate `workspace read` calls for files whose content is "
                "already included. Apply the fix with "
                "`./bin/workspace patch --stdin --json` and a standard unified "
                "git diff on stdin. Do not use Codex apply_patch format, and do "
                "not create or delete a temporary patch file. If you use a "
                "heredoc, use exactly `./bin/workspace patch --stdin --json "
                "<<'PATCH'` and put the diff content literally; because the "
                "`PATCH` delimiter is quoted, do not shell-escape `$` values in "
                "documentation text. Build the unified diff from the exact "
                "returned content, including complete JSON closing-brace context "
                "and the final newline, so `git apply` accepts the first patch. "
                "Run the test command "
                f"exactly as `./bin/workspace run \"{TEST_COMMAND}\" --json`; "
                "do not use a `--` separator for `workspace run`. This is a "
                "small two-file task; after the tests pass, skip impact analysis "
                "and finish with `./bin/workspace diff --json`."
            ),
            create_repo=create_policy_fixture_repo,
            expected_changed_files=(
                "config/discount_policy.json",
                "docs/discount_policy.md",
            ),
        ),
        "rollback_recovery": TaskSpec(
            name="rollback_recovery",
            prompt=rollback_prompt,
            workspace_extra=(
                "Use `./bin/workspace` for workspace observation, patching, "
                "rollback, and verification whenever possible. The command "
                "syntax below is complete; do not spend time running "
                "`workspace --help`, command-specific `--help`, or inspecting "
                "`.workspace` metadata, and do not run `workspace status` "
                "before applying the proposed patch. Use this sequence: "
                "`./bin/workspace patch --description \"Validate proposed "
                "late-fee patch\" docs/proposed_late_fee_fix.patch --json`; "
                f"`./bin/workspace run \"{TEST_COMMAND}\" --json`. If the tests "
                "fail, read the patch response's `data.transaction_id` and run "
                "`./bin/workspace rollback <transaction_id> --json`. After "
                "rollback, run exactly `./bin/workspace read "
                "docs/proposed_late_fee_fix.patch --json` to inspect the bad "
                "patch hunks; do not read `src/billing.py` or `docs/billing.md` "
                "unless rollback itself failed. Reuse the proposed patch hunks "
                "to change only the cap and documentation text, leaving "
                "`LATE_FEE_RATE_CENTS = 150` unchanged. Apply the correct patch "
                "with "
                "`./bin/workspace patch --stdin --description \"Apply correct "
                "late-fee cap fix\" --json` and a standard unified git diff on "
                "stdin. Do not use Codex apply_patch format, and do not create "
                "or delete a temporary patch file. If you use a heredoc, use "
                "exactly `./bin/workspace patch --stdin --description \"Apply "
                "correct late-fee cap fix\" --json <<'PATCH'` and put the diff "
                "content literally; because the `PATCH` delimiter is quoted, do "
                "not shell-escape `$` values in documentation text. The correct "
                "patch must keep the daily rate at 150 cents and set the cap to "
                "1500 cents / $15.00. Run the "
                "test command exactly as `./bin/workspace run \""
                f"{TEST_COMMAND}\" --json`; do not use a `--` separator for "
                "`workspace run`. Then finish with `./bin/workspace diff --json`."
            ),
            create_repo=create_rollback_fixture_repo,
            expected_changed_files=("docs/billing.md", "src/billing.py"),
        ),
        "invoice_tax_sync": TaskSpec(
            name="invoice_tax_sync",
            prompt=invoice_tax_prompt,
            workspace_extra=(
                "Use `./bin/workspace` for workspace observation and verification. "
                "The command syntax below is complete; do not spend time running "
                "`workspace --help`, command-specific `--help`, or inspecting "
                "`.workspace` metadata. Do not run `workspace status` or "
                "`workspace index cochange` before the related-file query; "
                "`--ensure-index` creates or refreshes the co-change index in "
                "the same command. Start with "
                "`./bin/workspace related tests/test_invoice_pipeline.py --by "
                "cochange --ensure-index --max-commits 1000 --rank hybrid --max-results 4 "
                "--include-content --max-content-files 4 --json` to find and "
                "read the policy, label, and documentation files that usually "
                "change with this test. Use returned `data.included_content` "
                "instead of separate `workspace read` calls for files whose "
                "content is already included. Apply the fix with "
                "`./bin/workspace patch --stdin --json` and a standard unified "
                "git diff on stdin. Do not use Codex apply_patch format, and do "
                "not create or delete a temporary patch file. If you use a "
                "heredoc, use exactly `./bin/workspace patch --stdin --json "
                "<<'PATCH'` and put the diff content literally. Run the test command "
                f"exactly as `./bin/workspace run \"{TEST_COMMAND}\" --json`; "
                "do not use a `--` separator for `workspace run`. Then finish with "
                "`./bin/workspace impact --diff --by cochange --ensure-index "
                "--max-commits 1000 --rank hybrid --json` and "
                "`./bin/workspace diff --json`."
            ),
            create_repo=create_invoice_tax_fixture_repo,
            expected_changed_files=(
                "config/invoice_labels.json",
                "config/tax_regions.json",
                "docs/invoice_templates.md",
                "docs/tax_policy.md",
            ),
        ),
        "subscription_rollout_sync": TaskSpec(
            name="subscription_rollout_sync",
            prompt=subscription_rollout_prompt,
            workspace_extra=(
                "Use `./bin/workspace` for workspace observation and verification. "
                "The command syntax below is complete; do not spend time running "
                "`workspace --help`, command-specific `--help`, or inspecting "
                "`.workspace` metadata. Do not run `workspace status` or "
                "`workspace index cochange` before the related-file query; "
                "`--ensure-index` creates or refreshes the co-change index in "
                "the same command. Start with "
                "`./bin/workspace related tests/test_plan_rollout.py --by "
                "cochange --ensure-index --max-commits 1000 --rank hybrid "
                "--max-results 6 --include-content --max-content-files 6 --json` "
                "to find and read the plan limit, feature flag, billing catalog, "
                "documentation, and template files that usually change with this "
                "test. Use returned `data.included_content` instead of separate "
                "`workspace read` calls for files whose content is already "
                "included. Apply the fix with "
                "`./bin/workspace patch --stdin --json` and a standard unified "
                "git diff on stdin. Do not use Codex apply_patch format, and do "
                "not create or delete a temporary patch file. If you use a "
                "heredoc, use exactly `./bin/workspace patch --stdin --json "
                "<<'PATCH'` and put the diff content literally. Run the test command "
                f"exactly as `./bin/workspace run \"{TEST_COMMAND}\" --json`; "
                "do not use a `--` separator for `workspace run`. Then finish with "
                "`./bin/workspace impact --diff --by cochange --ensure-index "
                "--max-commits 1000 --rank hybrid --json` and "
                "`./bin/workspace diff --json`."
            ),
            create_repo=create_subscription_rollout_fixture_repo,
            expected_changed_files=(
                "config/billing_catalog.json",
                "config/feature_flags.json",
                "config/plan_limits.json",
                "docs/api_contract.md",
                "docs/plans.md",
                "templates/enterprise_welcome_email.txt",
            ),
        ),
    }


def condition_prompts(task: TaskSpec) -> list[Condition]:
    return [
        Condition(
            name="shell_only",
            prompt=(
                task.prompt
                + "\nDo not use the `workspace` command. Use ordinary shell tools."
            ),
        ),
        Condition(
            name="workspace_cli",
            prompt=task.prompt + "\n" + task.workspace_extra,
        ),
    ]


def run_codex_condition(
    condition: Condition,
    *,
    repo: Path,
    codex_binary: str,
    timeout_seconds: int,
) -> dict[str, Any]:
    command = [
        codex_binary,
        "exec",
        "--json",
        "--ephemeral",
        "--sandbox",
        "workspace-write",
        "-C",
        str(repo),
        "-",
    ]
    started = time.monotonic()
    try:
        result = run_command(
            command,
            cwd=repo,
            input_text=condition.prompt,
            timeout_seconds=timeout_seconds,
        )
        timed_out = False
    except subprocess.TimeoutExpired as error:
        elapsed = time.monotonic() - started
        return {
            "condition": condition.name,
            "timed_out": True,
            "elapsed_seconds": round(elapsed, 3),
            "codex_exit_code": None,
            "codex_stdout": error.stdout or "",
            "codex_stderr": error.stderr or "",
        }
    elapsed = time.monotonic() - started
    return {
        "condition": condition.name,
        "timed_out": timed_out,
        "elapsed_seconds": round(elapsed, 3),
        "codex_exit_code": result.returncode,
        "codex_stdout": result.stdout,
        "codex_stderr": result.stderr,
    }


def load_jsonl_events(text: str) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            events.append(value)
    return events


def walk_values(value: Any) -> list[Any]:
    values = [value]
    if isinstance(value, dict):
        for item in value.values():
            values.extend(walk_values(item))
    elif isinstance(value, list):
        for item in value:
            values.extend(walk_values(item))
    return values


def command_like_values(events: list[dict[str, Any]]) -> list[str]:
    commands: list[str] = []
    for event in events:
        for value in walk_values(event):
            if not isinstance(value, dict):
                continue
            for key in ("cmd", "command"):
                command = value.get(key)
                if isinstance(command, str) and command:
                    commands.append(command)
                elif isinstance(command, list) and all(
                    isinstance(part, str) for part in command
                ):
                    commands.append(" ".join(command))
    deduped: list[str] = []
    for command in commands:
        if command not in deduped:
            deduped.append(command)
    return deduped


def count_workspace_log_entries(repo: Path) -> int:
    log_path = repo / ".workspace" / "log.jsonl"
    if not log_path.is_file():
        return 0
    return sum(1 for line in log_path.read_text(encoding="utf-8").splitlines() if line)


def workspace_operation_counts(repo: Path) -> dict[str, int]:
    log_path = repo / ".workspace" / "log.jsonl"
    if not log_path.is_file():
        return {}
    counts: dict[str, int] = {}
    for line in log_path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        try:
            entry = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(entry, dict):
            continue
        op = entry.get("op")
        if isinstance(op, str):
            counts[op] = counts.get(op, 0) + 1
    return counts


def collect_condition_result(
    raw: dict[str, Any],
    repo: Path,
    *,
    expected_changed_files: tuple[str, ...] = (),
) -> dict[str, Any]:
    events = load_jsonl_events(str(raw.get("codex_stdout", "")))
    commands = command_like_values(events)
    operation_counts = workspace_operation_counts(repo)
    test = run_command(
        ["sh", "-c", TEST_COMMAND],
        cwd=repo,
        timeout_seconds=60,
    )
    diff_name_only = run_command(["git", "diff", "--name-only"], cwd=repo)
    diff_stat = run_command(["git", "diff", "--stat"], cwd=repo)
    diff_patch = run_command(["git", "diff"], cwd=repo)
    final_message = ""
    for event in events:
        item = event.get("item")
        if isinstance(item, dict) and item.get("type") == "agent_message":
            text = item.get("text")
            if isinstance(text, str):
                final_message = text

    changed_files = [line for line in diff_name_only.stdout.splitlines() if line.strip()]
    return {
        **{key: value for key, value in raw.items() if not key.startswith("codex_")},
        "codex_exit_code": raw.get("codex_exit_code"),
        "event_count": len(events),
        "event_type_counts": count_by_key(events, "type"),
        "command_count": len(commands),
        "workspace_command_count": sum(
            1 for command in commands if "workspace" in command
        ),
        "commands": commands,
        "workspace_log_entries": count_workspace_log_entries(repo),
        "workspace_operation_counts": operation_counts,
        "workspace_rollback_count": operation_counts.get("rollback", 0),
        "test_exit_code": test.returncode,
        "test_passed": test.returncode == 0,
        "test_stdout": test.stdout,
        "test_stderr": test.stderr,
        "changed_files": changed_files,
        "expected_changed_files": list(expected_changed_files),
        "changed_files_match_expected": (
            sorted(changed_files) == sorted(expected_changed_files)
            if expected_changed_files
            else None
        ),
        "diff_stat": diff_stat.stdout.strip(),
        "diff_patch": diff_patch.stdout,
        "final_message": final_message,
    }


def write_condition_artifacts(
    output_dir: Path,
    condition: str,
    result: dict[str, Any],
    repo: Path,
) -> None:
    (output_dir / f"{condition}.diff").write_text(
        str(result.get("diff_patch", "")),
        encoding="utf-8",
    )
    (output_dir / f"{condition}.commands.txt").write_text(
        "\n".join(result.get("commands", [])) + "\n",
        encoding="utf-8",
    )
    log_path = repo / ".workspace" / "log.jsonl"
    if log_path.is_file():
        shutil.copyfile(log_path, output_dir / f"{condition}.workspace-log.jsonl")


def count_by_key(events: list[dict[str, Any]], key: str) -> dict[str, int]:
    counts: dict[str, int] = {}
    for event in events:
        value = event.get(key)
        if isinstance(value, str):
            counts[value] = counts.get(value, 0) + 1
    return counts


def render_markdown(summary: dict[str, Any]) -> str:
    lines = [
        "# Codex Workspace Pilot",
        "",
        f"- task: `{summary['task']}`",
        f"- generated_at: `{summary['generated_at']}`",
        f"- codex: `{summary['codex_binary']}`",
        f"- workspace: `{summary['workspace_binary']}`",
        "",
        "| condition | passed | seconds | commands | workspace commands | workspace log entries | rollback ops | changed files |",
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |",
    ]
    for result in summary["results"]:
        changed = ", ".join(result["changed_files"])
        lines.append(
            "| {condition} | {passed} | {seconds} | {commands} | {workspace_commands} | "
            "{log_entries} | {rollback_ops} | {changed} |".format(
                condition=result["condition"],
                passed=str(result["test_passed"]).lower(),
                seconds=result["elapsed_seconds"],
                commands=result["command_count"],
                workspace_commands=result["workspace_command_count"],
                log_entries=result["workspace_log_entries"],
                rollback_ops=result.get("workspace_rollback_count", 0),
                changed=changed,
            )
        )
    lines.append("")
    lines.append("## Interpretation")
    lines.append("")
    results = {result["condition"]: result for result in summary["results"]}
    shell = results.get("shell_only")
    workspace = results.get("workspace_cli")
    if shell and workspace and shell["test_passed"] and workspace["test_passed"]:
        delta = workspace["elapsed_seconds"] - shell["elapsed_seconds"]
        if delta > 0:
            lines.append(
                "Both conditions solved this pilot task. On this pilot fixture, "
                f"`workspace_cli` took {delta:.3f}s longer than `shell_only`, which "
                "is evidence of overhead rather than an efficiency win for this task."
            )
        elif delta < 0:
            lines.append(
                "Both conditions solved this pilot task. On this pilot fixture, "
                f"`workspace_cli` finished {-delta:.3f}s faster than `shell_only`. "
                "This is a positive pilot result, but it is still a single run and "
                "not a statistically powered efficiency claim."
            )
        else:
            lines.append(
                "Both conditions solved this pilot task in the same elapsed time. "
                "This is neutral timing evidence for this single run."
            )
        lines.append("")
        lines.append(
            "The useful result is methodological: the `workspace_cli` run used "
            f"{workspace['workspace_command_count']} workspace commands and wrote "
            f"{workspace['workspace_log_entries']} operation-log entries, so real "
            "Codex-in-the-loop measurements are now reproducible."
        )
    else:
        lines.append(
            "At least one condition did not pass. Inspect the JSONL, stderr, diff, "
            "and workspace-log artifacts before treating this pilot as evidence."
        )
    lines.append("")
    lines.append("## Notes")
    lines.append("")
    lines.append(
        "This is a pilot, not a statistically powered result. It verifies that the "
        "evaluation path can run real Codex turns against controlled repositories."
    )
    lines.append("")
    return "\n".join(lines)


def run_pilot(args: argparse.Namespace) -> dict[str, Any]:
    workspace_binary = resolve_workspace_binary(args.workspace_bin)
    codex_binary = args.codex_binary
    tasks = task_specs()
    task = tasks[args.task]
    output_dir = args.output_dir.resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="workspace-codex-pilot-") as tmp:
        tmp_root = Path(tmp)
        results = []
        for condition in condition_prompts(task):
            repo = tmp_root / condition.name
            task.create_repo(repo, workspace_binary)
            raw = run_codex_condition(
                condition,
                repo=repo,
                codex_binary=codex_binary,
                timeout_seconds=args.timeout_seconds,
            )
            (output_dir / f"{condition.name}.jsonl").write_text(
                str(raw.get("codex_stdout", "")),
                encoding="utf-8",
            )
            (output_dir / f"{condition.name}.stderr.txt").write_text(
                str(raw.get("codex_stderr", "")),
                encoding="utf-8",
            )
            result = collect_condition_result(
                raw,
                repo,
                expected_changed_files=task.expected_changed_files,
            )
            write_condition_artifacts(output_dir, condition.name, result, repo)
            results.append(result)

    summary = {
        "schema_version": 1,
        "task": task.name,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "codex_binary": codex_binary,
        "workspace_binary": str(workspace_binary),
        "results": results,
    }
    (output_dir / "summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    (output_dir / "summary.md").write_text(
        render_markdown(summary),
        encoding="utf-8",
    )
    return summary


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--task",
        choices=sorted(task_specs()),
        default=DEFAULT_TASK,
        help="pilot task to run",
    )
    parser.add_argument(
        "--codex-binary",
        default=os.environ.get("CODEX_BINARY", "codex"),
        help="codex executable to run",
    )
    parser.add_argument(
        "--workspace-bin",
        type=Path,
        help="workspace binary to copy into the pilot repositories",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=DEFAULT_OUTPUT_DIR,
        help="directory for pilot JSONL logs and summary files",
    )
    parser.add_argument(
        "--timeout-seconds",
        type=int,
        default=420,
        help="timeout for each Codex condition",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        summary = run_pilot(args)
    except Exception as error:
        print(f"codex workspace pilot failed: {error}", file=sys.stderr)
        return 1
    print(render_markdown(summary), end="")
    print(f"wrote codex workspace pilot artifacts to {args.output_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
