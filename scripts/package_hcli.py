#!/usr/bin/env python3
"""Package a built HCLI executable; uses only the Python standard library."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import tarfile
import tempfile
import zipfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("artifact")
    parser.add_argument("--commit", required=True)
    parser.add_argument("--output", type=Path, default=Path("dist"))
    args = parser.parse_args()
    repo = Path(__file__).resolve().parent.parent
    windows = args.binary.suffix.lower() == ".exe"
    executable = "hcli.exe" if windows else "hcli"
    args.output.mkdir(parents=True, exist_ok=True)
    suffix = ".zip" if windows else ".tar.gz"
    archive = args.output / (args.artifact + suffix)
    with tempfile.TemporaryDirectory(prefix="hcli-package-") as temp:
        staging = Path(temp) / args.artifact
        staging.mkdir()
        shutil.copy2(args.binary, staging / executable)
        (staging / executable).chmod(0o755)
        for license_file in repo.glob("LICENSE*"):
            if license_file.is_file():
                shutil.copy2(license_file, staging / license_file.name)
        shutil.copy2(repo / "docs/hcli-setup.md", staging / "README.md")
        (staging / "BUILD.json").write_text(json.dumps({
            "name": "HCLI",
            "platform": args.artifact.removeprefix("hcli-"),
            "commit": args.commit,
            "features": "--no-default-features",
            "signed": False,
        }, indent=2) + "\n")
        if windows:
            with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as bundle:
                for path in sorted(staging.iterdir()):
                    bundle.write(path, f"{args.artifact}/{path.name}")
        else:
            with tarfile.open(archive, "w:gz") as bundle:
                bundle.add(staging, arcname=args.artifact)
    checksum = hashlib.sha256()
    with archive.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            checksum.update(chunk)
    digest = checksum.hexdigest()
    archive.with_name(archive.name + ".sha256").write_text(f"{digest}  {archive.name}\n")
    print(f"Packaged {archive} ({archive.stat().st_size:,} bytes)")


if __name__ == "__main__":
    main()
