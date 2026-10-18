#!/usr/bin/env python3
"""The VS Code extension packaged as a .vsix, with Python's standard library
alone: `python3 editors/vscode/package_vsix.py <version> <out.vsix>`.

A .vsix is a zip in the Open Packaging Conventions: `[Content_Types].xml`,
`extension.vsixmanifest` (the gallery's metadata, written here from
package.json as `vsce` writes it), and the extension's files under
`extension/`. The extension depends on no npm package, so packaging it needs
neither npm nor `vsce` — nor Node.js, which Crow's release no longer installs.
"""

import json
import os
import sys
import zipfile
from xml.sax.saxutils import escape

HERE = os.path.dirname(os.path.abspath(__file__))

# what the extension ships, by its path here and its path in the package
# (vsce's own names: the licence as LICENSE.txt, the readme lowercased)
FILES = [
    ("extension.js", "extension/extension.js"),
    ("README.md", "extension/readme.md"),
    ("LICENSE", "extension/LICENSE.txt"),
    ("icon.png", "extension/icon.png"),
    ("language-configuration.json", "extension/language-configuration.json"),
    ("syntaxes/crow.tmLanguage.json", "extension/syntaxes/crow.tmLanguage.json"),
]

TYPES = {
    ".js": "application/javascript",
    ".json": "application/json",
    ".md": "text/markdown",
    ".png": "image/png",
    ".txt": "text/plain",
    ".vsixmanifest": "text/xml",
}


def manifest(pkg):
    """The gallery's metadata, as `vsce package` writes it."""
    repo = pkg.get("repository", {}).get("url", "")
    if repo and not repo.endswith(".git"):
        repo += ".git"
    tags = list(pkg.get("keywords", []))
    for language in pkg.get("contributes", {}).get("languages", []):
        tags += language.get("aliases", [])
        tags += ["__ext_" + e.lstrip(".") for e in language.get("extensions", [])]
    banner = pkg.get("galleryBanner", {})
    props = [
        ("Microsoft.VisualStudio.Code.Engine", pkg["engines"]["vscode"]),
        ("Microsoft.VisualStudio.Code.ExtensionDependencies", ""),
        ("Microsoft.VisualStudio.Code.ExtensionPack", ""),
        ("Microsoft.VisualStudio.Code.ExtensionKind", "workspace"),
        ("Microsoft.VisualStudio.Code.LocalizedLanguages", ""),
        ("Microsoft.VisualStudio.Code.EnabledApiProposals", ""),
        ("Microsoft.VisualStudio.Code.ExecutesCode", "true"),
        ("Microsoft.VisualStudio.Services.Links.Source", repo),
        ("Microsoft.VisualStudio.Services.Links.Getstarted", repo),
        ("Microsoft.VisualStudio.Services.Links.GitHub", repo),
        ("Microsoft.VisualStudio.Services.Links.Support", pkg.get("bugs", {}).get("url", "")),
        ("Microsoft.VisualStudio.Services.Links.Learn", pkg.get("homepage", "")),
        ("Microsoft.VisualStudio.Services.Branding.Color", banner.get("color", "")),
        ("Microsoft.VisualStudio.Services.Branding.Theme", banner.get("theme", "")),
        ("Microsoft.VisualStudio.Services.GitHubFlavoredMarkdown", "true"),
        ("Microsoft.VisualStudio.Services.Content.Pricing", "Free"),
    ]
    a = lambda s: escape(str(s), {'"': "&quot;"})
    lines = [
        '<?xml version="1.0" encoding="utf-8"?>',
        '<PackageManifest Version="2.0.0" xmlns="http://schemas.microsoft.com/developer/vsx-schema/2011" xmlns:d="http://schemas.microsoft.com/developer/vsx-schema-design/2011">',
        "  <Metadata>",
        '    <Identity Language="en-US" Id="{}" Version="{}" Publisher="{}" />'.format(a(pkg["name"]), a(pkg["version"]), a(pkg["publisher"])),
        "    <DisplayName>{}</DisplayName>".format(escape(pkg.get("displayName", pkg["name"]))),
        '    <Description xml:space="preserve">{}</Description>'.format(escape(pkg.get("description", ""))),
        "    <Tags>{}</Tags>".format(escape(",".join(tags))),
        "    <Categories>{}</Categories>".format(escape(",".join(pkg.get("categories", [])))),
        "    <GalleryFlags>Public</GalleryFlags>",
        "    <Properties>",
    ]
    lines += ['      <Property Id="{}" Value="{}" />'.format(k, a(v)) for k, v in props]
    lines += [
        "    </Properties>",
        "    <License>extension/LICENSE.txt</License>",
        "    <Icon>extension/icon.png</Icon>",
        "  </Metadata>",
        "  <Installation>",
        '    <InstallationTarget Id="Microsoft.VisualStudio.Code"/>',
        "  </Installation>",
        "  <Dependencies/>",
        "  <Assets>",
        '    <Asset Type="Microsoft.VisualStudio.Code.Manifest" Path="extension/package.json" Addressable="true" />',
        '    <Asset Type="Microsoft.VisualStudio.Services.Content.Details" Path="extension/readme.md" Addressable="true" />',
        '    <Asset Type="Microsoft.VisualStudio.Services.Content.License" Path="extension/LICENSE.txt" Addressable="true" />',
        '    <Asset Type="Microsoft.VisualStudio.Services.Icons.Default" Path="extension/icon.png" Addressable="true" />',
        "  </Assets>",
        "</PackageManifest>",
    ]
    return "\n".join(lines) + "\n"


def content_types(names):
    exts = sorted({os.path.splitext(n)[1] for n in names} | {".vsixmanifest"})
    defaults = "".join('<Default Extension="{}" ContentType="{}"/>'.format(e, TYPES.get(e, "application/octet-stream")) for e in exts)
    return '<?xml version="1.0" encoding="utf-8"?>\n<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">{}</Types>\n'.format(defaults)


def main():
    if len(sys.argv) != 3:
        sys.exit("usage: package_vsix.py <version> <out.vsix>")
    version, out = sys.argv[1], sys.argv[2]
    with open(os.path.join(HERE, "package.json"), encoding="utf-8") as f:
        pkg = json.load(f)
    # (the extension's version is Crow's)
    pkg["version"] = version
    if pkg.get("dependencies"):
        sys.exit("the extension depends on no npm package: package.json lists {}".format(", ".join(pkg["dependencies"])))
    names = [inside for _, inside in FILES] + ["extension/package.json"]
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("extension.vsixmanifest", manifest(pkg))
        z.writestr("[Content_Types].xml", content_types(names))
        z.writestr("extension/package.json", json.dumps(pkg, indent=2) + "\n")
        for here, inside in FILES:
            z.write(os.path.join(HERE, here), inside)
    print("wrote {} ({} files)".format(out, len(names) + 2))


if __name__ == "__main__":
    main()
