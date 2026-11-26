#!/usr/bin/env python3
"""Mirror one release to Gitee, splitting the installer below its 100 MB cap."""
import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

import requests

PART_BYTES = 60 * 1024 * 1024


def fail(message: str) -> None:
    print(message, file=sys.stderr)
    raise SystemExit(1)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag", required=True)
    parser.add_argument("--assets", required=True, type=Path)
    parser.add_argument("--token", required=True)
    parser.add_argument("--repo", default="zhhpan/literature-reader")
    parser.add_argument("--github-token", default="")
    args = parser.parse_args()

    version = args.tag.removeprefix("v")
    installer = next(args.assets.glob(f"LiteratureReader-Setup-{version}-x64.exe"), None)
    if installer is None:
        fail(f"installer for {args.tag} is missing from {args.assets}")
    data = installer.read_bytes()
    digest = hashlib.sha256(data).hexdigest()

    parts = []
    for index, offset in enumerate(range(0, len(data), PART_BYTES), start=1):
        chunk = data[offset:offset + PART_BYTES]
        name = f"{installer.name}.part{index}"
        path = args.assets / name
        path.write_bytes(chunk)
        parts.append((name, path, len(chunk)))

    api = f"https://gitee.com/api/v5/repos/{args.repo}"
    session = requests.Session()
    response = session.get(f"{api}/releases", params={"access_token": args.token}, timeout=60)
    response.raise_for_status()
    existing = next((item for item in response.json() if item.get("tag_name") == args.tag), None)
    if existing:
        deleted = session.delete(f"{api}/releases/{existing['id']}",
                                 params={"access_token": args.token}, timeout=60)
        if deleted.status_code not in (200, 204, 404):
            fail(f"could not delete the old Gitee release: {deleted.status_code} {deleted.text}")

    release = session.post(f"{api}/releases", data={
        "access_token": args.token,
        "tag_name": args.tag,
        "name": args.tag,
        "body": f"Gitee mirror of https://github.com/{args.repo}/releases/tag/{args.tag}",
        "prerelease": "false",
        "target_commitish": args.tag,
    }, timeout=60)
    if release.status_code not in (200, 201):
        fail(f"could not create the Gitee release: {release.status_code} {release.text}")
    release_id = release.json()["id"]

    uploaded = {}
    for name, path, size in parts:
        with path.open("rb") as handle:
            upload = session.post(f"{api}/releases/{release_id}/attach_files",
                                  params={"access_token": args.token},
                                  files={"file": (name, handle)}, timeout=600)
        if upload.status_code not in (200, 201):
            fail(f"could not upload {name}: {upload.status_code} {upload.text[:300]}")
        body = upload.json()
        if not body.get("browser_download_url"):
            fail(f"Gitee did not return a download URL for {name}")
        uploaded[name] = body
        print(f"uploaded {name} ({size} bytes)")

    manifest = {
        "tag_name": args.tag,
        "draft": False,
        "prerelease": False,
        "assets": [{
            "name": installer.name,
            "state": "uploaded",
            "size": len(data),
            "browser_download_url": f"https://gitee.com/{args.repo}/releases/download/{args.tag}/{installer.name}",
            "digest": f"sha256:{digest}",
            "parts": [{
                "name": name,
                "size": size,
                "url": uploaded[name]["browser_download_url"],
            } for name, _, size in parts],
        }],
    }
    root = Path(__file__).resolve().parent.parent
    (root / "gitee-update.json").write_text(json.dumps(manifest, separators=(",", ":")), encoding="utf-8")
    print(f"wrote {root / 'gitee-update.json'}")

    token = args.token
    gitee_url = f"https://oauth2:{token}@gitee.com/{args.repo}.git"
    subprocess.run(["git", "-C", str(root), "push", "--force", gitee_url,
                    "HEAD:refs/heads/master", f"refs/tags/{args.tag}:refs/tags/{args.tag}"],
                   check=True)
    subprocess.run(["git", "-C", str(root), "add", "gitee-update.json"], check=True)
    if subprocess.run(["git", "-C", str(root), "diff", "--cached", "--quiet"]).returncode != 0:
        subprocess.run(["git", "-C", str(root), "-c", "user.name=github-actions[bot]",
                        "-c", "user.email=41898282+github-actions[bot]@users.noreply.github.com",
                        "commit", "-m", f"Update the Gitee manifest for {args.tag}"], check=True)
        subprocess.run(["git", "-C", str(root), "push", gitee_url, "HEAD:refs/heads/master"], check=True)
        if args.github_token:
            github_url = f"https://x-access-token:{args.github_token}@github.com/{args.repo}.git"
            subprocess.run(["git", "-C", str(root), "push", github_url, "HEAD:refs/heads/main"], check=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
