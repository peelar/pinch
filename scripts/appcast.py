#!/usr/bin/env python3
"""Create the stable release's Sparkle feed, published alongside its archive."""
import argparse
import base64
from pathlib import Path
import re
import xml.etree.ElementTree as ET

SPARKLE = "http://www.andymatuschak.org/xml-namespaces/sparkle"
ET.register_namespace("sparkle", SPARKLE)


def appcast(repository, version, archive, signature, notes):
    if not re.fullmatch(r"[\w.-]+/[\w.-]+", repository):
        raise ValueError("invalid repository")
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError("stable release requires a major.minor.patch version")
    if len(base64.b64decode(signature, validate=True)) != 64:
        raise ValueError("invalid Ed25519 signature")
    rss = ET.Element("rss", version="2.0")
    channel = ET.SubElement(rss, "channel")
    ET.SubElement(channel, "title").text = "Pinch updates"
    item = ET.SubElement(channel, "item")
    ET.SubElement(item, "title").text = f"Pinch {version}"
    ET.SubElement(item, f"{{{SPARKLE}}}version").text = version
    ET.SubElement(item, f"{{{SPARKLE}}}shortVersionString").text = version
    ET.SubElement(item, f"{{{SPARKLE}}}minimumSystemVersion").text = "13.0"
    ET.SubElement(item, "description", {f"{{{SPARKLE}}}descriptionFormat": "plain-text"}).text = notes
    ET.SubElement(item, "enclosure", {
        "url": f"https://github.com/{repository}/releases/download/v{version}/{archive.name}",
        "length": str(archive.stat().st_size),
        "type": "application/octet-stream",
        f"{{{SPARKLE}}}edSignature": signature,
    })
    ET.indent(rss)
    return ET.tostring(rss, encoding="utf-8", xml_declaration=True)


def release_notes(changelog, version):
    text = changelog.read_text()
    match = re.search(rf"^## {re.escape(version)}\s*\n(.*?)(?=^## |\Z)", text, re.M | re.S)
    if not match:
        raise ValueError(f"missing changelog entry for {version}")
    return match.group(1).strip()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--repository", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--signature", required=True)
    parser.add_argument("--changelog", type=Path, default=Path("CHANGELOG.md"))
    parser.add_argument("--output", type=Path, default=Path("dist/appcast.xml"))
    args = parser.parse_args()
    args.output.write_bytes(appcast(args.repository, args.version, args.archive,
                                  args.signature, release_notes(args.changelog, args.version)))
