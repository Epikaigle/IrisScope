#!/usr/bin/env python3
"""Check GitHub Pages resources without a bundler or a network connection."""
from html.parser import HTMLParser
from pathlib import Path
import re
import subprocess
from urllib.parse import unquote, urlsplit

SITE = Path(__file__).resolve().parents[1] / "docs"


class Page(HTMLParser):
    def __init__(self):
        super().__init__()
        self.ids = set()
        self.references = []
        self.errors = []

    def handle_starttag(self, tag, attrs):
        attributes = dict(attrs)
        if "id" in attributes:
            identifier = attributes["id"]
            if identifier in self.ids:
                self.errors.append(f"Duplicate id: {identifier}")
            self.ids.add(identifier)
        if tag == "img" and "alt" not in attributes:
            self.errors.append("Image missing alt text")
        for name in ("href", "src", "poster"):
            if attributes.get(name):
                self.references.append((attributes[name], tag != "a"))
        if attributes.get("srcset"):
            for candidate in attributes["srcset"].split(","):
                parts = candidate.strip().split()
                if parts:
                    self.references.append((parts[0], True))
        for name in ("aria-controls", "aria-labelledby", "aria-describedby"):
            for identifier in attributes.get(name, "").split():
                self.references.append((f"#{identifier}", False))


def main():
    errors = []
    pages = {}
    for document in SITE.rglob("*.html"):
        page = Page()
        page.feed(document.read_text(encoding="utf-8"))
        page.close()
        pages[document.resolve()] = page
        errors.extend(f"{document.name}: {error}" for error in page.errors)
    if (SITE / "index.html").resolve() not in pages:
        errors.append("Missing docs/index.html")

    def check_resource(reference, origin, local_only=True):
        parsed = urlsplit(reference)
        if parsed.scheme or parsed.netloc:
            if local_only:
                errors.append(f"External rendering resource: {origin.name}: {reference}")
            return
        decoded_path = unquote(parsed.path)
        if decoded_path.startswith("/"):
            errors.append(f"Root-relative path breaks project Pages: {reference}")
            return
        target = (origin.parent / decoded_path).resolve() if decoded_path else origin.resolve()
        if decoded_path and (not target.is_relative_to(SITE) or not target.is_file()):
            errors.append(f"Missing or escaping resource: {origin.name}: {reference}")
            return
        if parsed.fragment:
            page = pages.get(target)
            if page is not None and unquote(parsed.fragment) not in page.ids:
                errors.append(f"Missing anchor: {origin.name}: {reference}")
        elif reference == "#":
            errors.append(f"Placeholder link: {origin.name}: {reference}")

    for document, page in pages.items():
        for reference, local_only in page.references:
            check_resource(reference, document, local_only)
    for stylesheet in SITE.rglob("*.css"):
        source = stylesheet.read_text(encoding="utf-8")
        for reference in re.findall(r"url\(\s*['\"]?([^)'\"\s]+)", source):
            check_resource(reference, stylesheet)
        for reference in re.findall(r"@import\s+['\"]([^'\"]+)['\"]", source):
            check_resource(reference, stylesheet)
    # Vendor bundles already contain all their imports; only maintained code
    # needs import resolution and syntax checks here.
    for script in SITE.glob("*.js"):
        subprocess.run(["node", "--check", str(script)], check=True)
        source = script.read_text(encoding="utf-8")
        pattern = r"(?:from\s*|import\s*(?:\(\s*)?)['\"]([^'\"]+)['\"]"
        for reference in re.findall(pattern, source):
            if not reference.startswith(("./", "../")):
                errors.append(f"Non-relative module import: {script.name}: {reference}")
            check_resource(reference, script)
    for resource in SITE.rglob("*"):
        if resource.is_symlink():
            errors.append(f"GitHub Pages artifact cannot contain links: {resource.relative_to(SITE)}")
    if errors:
        raise SystemExit("\n".join(errors))
    identifiers = sum(len(page.ids) for page in pages.values())
    references = sum(len(page.references) for page in pages.values())
    print(f"Site OK: {identifiers} anchors and {references} HTML references; CSS and JS resources present.")


if __name__ == "__main__":
    main()
