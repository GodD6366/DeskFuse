#!/usr/bin/env python3
"""Export the transparent mark from the single application SVG source."""
from pathlib import Path
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parent.parent
SVG = "http://www.w3.org/2000/svg"
ET.register_namespace("", SVG)
source = ET.parse(ROOT / "resources/DeskFuse-icon.svg")
root = source.getroot()
# Export a transparent brand mark, not the dedicated menu bar template.
# All gradients and paths remain local vector data.
root.set("viewBox", "82 76 860 860")
root.find(f"{{{SVG}}}title").text = "DeskFuse transparent center mark"
root.find(f"{{{SVG}}}desc").text = (
    "Generated from DeskFuse-icon.svg by scripts/export-center-icon.py. "
    "Edit the application SVG, not this derived file."
)
tile = root.find(f"{{{SVG}}}g[@id='app-tile']")
if tile is None or root.find(f"{{{SVG}}}g[@id='center-mark']") is None:
    raise SystemExit("error: application SVG must contain app-tile and center-mark")
root.remove(tile)
output = ROOT / "resources/macos_center_icon.svg"
source.write(output, encoding="UTF-8", xml_declaration=True)
output.write_text("\n".join(line.rstrip() for line in output.read_text().splitlines()) + "\n")
