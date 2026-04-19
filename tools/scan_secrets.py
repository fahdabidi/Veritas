#!/usr/bin/env python3
"""Scan the repository for likely leaked secrets."""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from dataclasses import asdict, dataclass
from pathlib import Path


DEFAULT_EXCLUDED_DIRS = {
    ".git",
    ".hg",
    ".svn",
    ".idea",
    ".vscode",
    "__pycache__",
    ".venv",
    "venv",
    "node_modules",
    "dist",
    "build",
    "coverage",
    "target",
    "target-codex",
    "target-codexAOsozx",
}

DEFAULT_EXCLUDED_FILE_SUFFIXES = {
    ".png",
    ".jpg",
    ".jpeg",
    ".gif",
    ".webp",
    ".mp4",
    ".mkv",
    ".mov",
    ".avi",
    ".mp3",
    ".wav",
    ".zip",
    ".tar",
    ".gz",
    ".tgz",
    ".7z",
    ".pdf",
    ".exe",
    ".dll",
    ".so",
    ".dylib",
    ".class",
    ".jar",
    ".pdb",
    ".o",
    ".a",
    ".lib",
    ".rmeta",
    ".rlib",
    ".wasm",
}

SUSPICIOUS_FILE_NAMES = {
    ".env",
    ".env.local",
    ".env.production",
    ".env.development",
    ".env.test",
    "credentials",
    "id_rsa",
    "id_dsa",
    "id_ed25519",
    "authorized_keys",
}

FILENAME_PATTERNS = [
    re.compile(r".*\.pem$", re.IGNORECASE),
    re.compile(r".*\.p12$", re.IGNORECASE),
    re.compile(r".*\.pfx$", re.IGNORECASE),
    re.compile(r".*\.key$", re.IGNORECASE),
    re.compile(r".*\.crt$", re.IGNORECASE),
    re.compile(r".*\.cer$", re.IGNORECASE),
]

ALLOWLIST_MARKER = "secretscan:allow"
MAX_FILE_BYTES = 2 * 1024 * 1024


@dataclass
class Finding:
    rule_id: str
    severity: str
    path: str
    line: int
    kind: str
    message: str
    excerpt: str


@dataclass(frozen=True)
class Rule:
    rule_id: str
    severity: str
    kind: str
    message: str
    regex: re.Pattern[str]
    filename_only: bool = False


RULES = [
    Rule(
        "private-key-block",
        "critical",
        "content",
        "Private key material block detected.",
        re.compile(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----"),
    ),
    Rule(
        "aws-access-key-id",
        "high",
        "content",
        "AWS access key ID detected.",
        re.compile(r"\b(?:AKIA|ASIA|ABIA|ACCA|AGPA|AIDA|AIPA|ANPA|ANVA|AROA)[A-Z0-9]{16}\b"),
    ),
    Rule(
        "aws-secret-access-key",
        "critical",
        "content",
        "AWS secret access key assignment detected.",
        re.compile(
            r"(?i)\baws(?:_|-| )?secret(?:_|-| )?access(?:_|-| )?key\b"
            r"\s*[:=]\s*['\"]?([A-Za-z0-9/+=]{40})['\"]?"
        ),
    ),
    Rule(
        "github-token",
        "high",
        "content",
        "GitHub token detected.",
        re.compile(r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36,255}\b"),
    ),
    Rule(
        "github-fine-grained-token",
        "high",
        "content",
        "GitHub fine-grained token detected.",
        re.compile(r"\bgithub_pat_[A-Za-z0-9_]{20,255}\b"),
    ),
    Rule(
        "slack-token",
        "high",
        "content",
        "Slack token detected.",
        re.compile(r"\bxox(?:a|b|p|r|s)-[A-Za-z0-9-]{10,200}\b"),
    ),
    Rule(
        "jwt",
        "medium",
        "content",
        "JWT-like bearer token detected.",
        re.compile(r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9._-]{8,}\.[A-Za-z0-9._-]{8,}\b"),
    ),
    Rule(
        "generic-secret-assignment",
        "medium",
        "content",
        "Suspicious credential-style assignment detected.",
        re.compile(
            r"""(?ix)
            \b(
                secret|
                token|
                password|
                passwd|
                api[_-]?key|
                client[_-]?secret|
                access[_-]?key|
                private[_-]?key
            )\b
            [A-Za-z0-9_.-]{0,32}
            \s*[:=]\s*
            ['"]?
            ([A-Za-z0-9/+_=.-]{16,})
            ['"]?
            """
        ),
    ),
    Rule(
        "suspicious-secret-file",
        "medium",
        "filename",
        "Suspicious secret-bearing filename detected.",
        re.compile(r".*"),
        filename_only=True,
    ),
]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Scan the repository for likely leaked secrets."
    )
    parser.add_argument(
        "root",
        nargs="?",
        default=".",
        help="Repository root to scan. Defaults to current directory.",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Emit findings as JSON.",
    )
    parser.add_argument(
        "--fail-on-findings",
        action="store_true",
        help="Return exit code 1 when any findings are detected.",
    )
    parser.add_argument(
        "--max-file-bytes",
        type=int,
        default=MAX_FILE_BYTES,
        help=f"Skip files larger than this many bytes. Default: {MAX_FILE_BYTES}.",
    )
    return parser.parse_args()


def should_exclude_dir(name: str) -> bool:
    if name in DEFAULT_EXCLUDED_DIRS:
        return True
    return name.startswith(".cache")


def should_exclude_file(path: Path) -> bool:
    if path.suffix.lower() in DEFAULT_EXCLUDED_FILE_SUFFIXES:
        return True
    return False


def is_suspicious_filename(path: Path) -> bool:
    filename = path.name.lower()
    if filename in SUSPICIOUS_FILE_NAMES:
        return True
    return any(pattern.fullmatch(path.name) for pattern in FILENAME_PATTERNS)


def is_probably_text(data: bytes) -> bool:
    if b"\x00" in data:
        return False
    try:
        data.decode("utf-8")
        return True
    except UnicodeDecodeError:
        try:
            data.decode("utf-8", errors="ignore")
        except Exception:
            return False
    return True


def redact_excerpt(line: str, match: re.Match[str]) -> str:
    start, end = match.span()
    prefix = line[max(0, start - 24):start]
    suffix = line[end:end + 8]
    return f"{prefix}<redacted>{suffix}".strip()


def scan_file(path: Path, root: Path, max_file_bytes: int) -> list[Finding]:
    findings: list[Finding] = []

    if should_exclude_file(path):
        return findings

    try:
        size = path.stat().st_size
    except OSError:
        return findings

    if size > max_file_bytes:
        return findings

    if is_suspicious_filename(path):
        findings.append(
            Finding(
                rule_id="suspicious-secret-file",
                severity="medium",
                path=str(path.relative_to(root)),
                line=1,
                kind="filename",
                message="Suspicious secret-bearing filename detected.",
                excerpt=path.name,
            )
        )

    try:
        raw = path.read_bytes()
    except OSError:
        return findings

    if not is_probably_text(raw):
        return findings

    text = raw.decode("utf-8", errors="replace")
    for line_number, line in enumerate(text.splitlines(), start=1):
        if ALLOWLIST_MARKER in line:
            continue
        for rule in RULES:
            if rule.filename_only:
                continue
            match = rule.regex.search(line)
            if not match:
                continue
            findings.append(
                Finding(
                    rule_id=rule.rule_id,
                    severity=rule.severity,
                    path=str(path.relative_to(root)),
                    line=line_number,
                    kind=rule.kind,
                    message=rule.message,
                    excerpt=redact_excerpt(line, match),
                )
            )

    return findings


def walk_files(root: Path) -> list[Path]:
    files: list[Path] = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [name for name in dirnames if not should_exclude_dir(name)]
        current_dir = Path(dirpath)
        for filename in filenames:
            files.append(current_dir / filename)
    return files


def scan_repo(root: Path, max_file_bytes: int) -> list[Finding]:
    findings: list[Finding] = []
    for path in walk_files(root):
        findings.extend(scan_file(path, root, max_file_bytes))
    findings.sort(key=lambda item: (item.path, item.line, item.rule_id))
    return findings


def print_human(findings: list[Finding], root: Path) -> None:
    print(f"Scanned: {root}")
    if not findings:
        print("No likely leaked secrets found.")
        return

    print(f"Findings: {len(findings)}")
    for finding in findings:
        print(
            f"[{finding.severity}] {finding.rule_id} "
            f"{finding.path}:{finding.line} {finding.message}"
        )
        print(f"  {finding.excerpt}")


def main() -> int:
    args = parse_args()
    root = Path(args.root).resolve()
    findings = scan_repo(root, args.max_file_bytes)

    if args.json:
        payload = {
            "root": str(root),
            "findings": [asdict(finding) for finding in findings],
        }
        print(json.dumps(payload, indent=2))
    else:
        print_human(findings, root)

    if findings and args.fail_on_findings:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
