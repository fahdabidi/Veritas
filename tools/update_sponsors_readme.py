#!/usr/bin/env python3
from __future__ import annotations

import json
import os
import sys
import urllib.error
import urllib.request
from collections import defaultdict
from pathlib import Path


README_PATH = Path("README.md")
START_MARKER = "<!-- SPONSOR_ACKS:START -->"
END_MARKER = "<!-- SPONSOR_ACKS:END -->"
KNOWN_TIER_ORDER = [
    "Support Veritas",
    "Back the Roadmap",
    "Infrastructure Sponsor",
    "Project Sponsor",
]
QUERY = """
query($login: String!, $count: Int!) {
  user(login: $login) {
    sponsorshipsAsMaintainer(first: $count) {
      nodes {
        privacyLevel
        isOneTimePayment
        sponsorEntity {
          __typename
          ... on User {
            login
            name
            url
          }
          ... on Organization {
            login
            name
            url
          }
        }
        tier {
          name
          isOneTime
          monthlyPriceInDollars
        }
      }
    }
  }
}
""".strip()


def fetch_public_sponsors(login: str, token: str | None, count: int = 100) -> dict[str, list[tuple[str, str]]]:
    if not token:
        return {}

    payload = json.dumps(
        {"query": QUERY, "variables": {"login": login, "count": count}},
        separators=(",", ":"),
    ).encode("utf-8")
    request = urllib.request.Request(
        "https://api.github.com/graphql",
        data=payload,
        headers={
            "Authorization": f"bearer {token}",
            "Content-Type": "application/json",
            "Accept": "application/json",
            "User-Agent": "veritas-sponsor-sync",
        },
        method="POST",
    )

    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            body = json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"GitHub GraphQL request failed: {exc.code} {detail}") from exc

    if body.get("errors"):
        raise RuntimeError(f"GitHub GraphQL returned errors: {body['errors']}")

    user = (body.get("data") or {}).get("user")
    if not user:
        raise RuntimeError(f"Unable to load GitHub user '{login}' from Sponsors API.")

    groups: dict[str, list[tuple[str, str]]] = defaultdict(list)
    seen: set[tuple[str, str]] = set()

    for node in user["sponsorshipsAsMaintainer"]["nodes"]:
        if node.get("privacyLevel") != "PUBLIC":
            continue

        sponsor = node.get("sponsorEntity") or {}
        login_name = sponsor.get("login")
        display_name = sponsor.get("name") or login_name
        url = sponsor.get("url") or (f"https://github.com/{login_name}" if login_name else None)
        if not display_name or not url:
            continue

        tier = node.get("tier") or {}
        tier_name = tier.get("name")
        if not tier_name:
            tier_name = "One-time Sponsors" if node.get("isOneTimePayment") else "Sponsors"

        key = (tier_name, login_name or display_name)
        if key in seen:
            continue
        seen.add(key)

        groups[tier_name].append((display_name, url))

    for tier_name in groups:
        groups[tier_name].sort(key=lambda item: item[0].lower())

    return dict(groups)


def render_block(groups: dict[str, list[tuple[str, str]]]) -> str:
    lines = [
        START_MARKER,
        "## Sponsor Acknowledgments",
        "",
        "Veritas is sustained by sponsors who help fund architecture work, testing, security preparation, and prototype infrastructure.",
        "",
        "[![Sponsor Veritas](https://img.shields.io/badge/Sponsor-Veritas-ea4aaa?logo=githubsponsors&logoColor=white)](https://github.com/sponsors/fahdabidi)",
        "",
    ]

    if not groups:
        lines.extend(
            [
                "Become our first sponsor.",
                "",
                "If you'd like to support Veritas, use the GitHub `Sponsor` button or see [docs/FUNDING.md](docs/FUNDING.md).",
                END_MARKER,
            ]
        )
        return "\n".join(lines)

    lines.extend(["Public sponsors who opted in to be named appear below.", ""])

    ordered_tiers = [name for name in KNOWN_TIER_ORDER if name in groups]
    ordered_tiers.extend(sorted(name for name in groups if name not in KNOWN_TIER_ORDER))

    for tier_name in ordered_tiers:
        lines.append(f"### {tier_name}")
        lines.append("")
        for display_name, url in groups[tier_name]:
            lines.append(f"- [{display_name}]({url})")
        lines.append("")

    lines.extend(
        [
            "If you'd like to support Veritas, use the GitHub `Sponsor` button or see [docs/FUNDING.md](docs/FUNDING.md).",
            END_MARKER,
        ]
    )
    return "\n".join(lines)


def replace_block(readme_text: str, block: str) -> str:
    if START_MARKER in readme_text and END_MARKER in readme_text:
        start = readme_text.index(START_MARKER)
        end = readme_text.index(END_MARKER) + len(END_MARKER)
        return readme_text[:start] + block + readme_text[end:]

    anchor = "Support the project:\n- Use the GitHub `Sponsor` button\n- Funding policy: [docs/FUNDING.md](docs/FUNDING.md)\n- Sponsor tiers and acknowledgments: [docs/FUNDING.md](docs/FUNDING.md#suggested-github-sponsors-tiers)\n"
    if anchor in readme_text:
        insertion_point = readme_text.index(anchor) + len(anchor)
        return readme_text[:insertion_point] + "\n" + block + "\n" + readme_text[insertion_point:]

    raise RuntimeError("Unable to locate README sponsor section markers or insertion anchor.")


def main() -> int:
    login = os.environ.get("GH_SPONSORS_LOGIN", "fahdabidi")
    token = os.environ.get("GH_SPONSORS_TOKEN")

    readme_text = README_PATH.read_text(encoding="utf-8")
    groups = fetch_public_sponsors(login, token)
    updated = replace_block(readme_text, render_block(groups))

    if updated != readme_text:
        README_PATH.write_text(updated + ("" if updated.endswith("\n") else "\n"), encoding="utf-8")
        print(f"Updated {README_PATH} sponsor acknowledgments.")
    else:
        print(f"No sponsor acknowledgment changes needed in {README_PATH}.")

    return 0


if __name__ == "__main__":
    sys.exit(main())
