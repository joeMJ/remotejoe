#!/usr/bin/env python3
"""Aktualisiert die Tabelle der Rust-Abhängigkeiten in THIRD_PARTY.md (alle Plattformen und Merkmale,
Lizenzangabe laut Paketbeschreibung); der übrige Text der Datei bleibt unverändert.
Aufruf im Hauptordner des Repositorys:  python3 -I tools/third-party.py"""
import json
import subprocess

DATEI = "THIRD_PARTY.md"
meta = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--locked", "--all-features"], text=True))
eigene = {p["id"] for p in meta["packages"] if p["source"] is None}
zeilen = [f"| {n} | {v} | {l} |" for n, v, l in sorted({(p["name"], p["version"], p.get("license") or "unbekannt") for p in meta["packages"] if p["id"] not in eigene})]

text = open(DATEI, encoding="utf-8").read().split("\n")
kopf = text.index("| Paket | Version | Lizenz |")
ende = kopf + 2
while ende < len(text) and text[ende].startswith("|"):
    ende += 1
text[kopf + 2:ende] = zeilen
open(DATEI, "w", encoding="utf-8").write("\n".join(text))
print(f"{len(zeilen)} Pakete eingetragen")
