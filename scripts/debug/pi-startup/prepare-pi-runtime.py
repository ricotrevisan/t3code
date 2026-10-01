#!/usr/bin/env python3
"""Prepare an isolated Pi 0.99.2 runtime; never alter the source install."""
import argparse
import hashlib
import json
import pathlib
import shutil

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("source", type=pathlib.Path)
parser.add_argument("destination", type=pathlib.Path)
args = parser.parse_args()
source = args.source.resolve()
destination = args.destination.resolve()
if destination.exists() or source == destination or source in destination.parents:
    parser.error("destination must be a new directory outside the source install")
if json.loads((source / "package.json").read_text())["version"] != "0.99.2":
    parser.error("this repair is pinned to Pi 0.99.2")
relative = pathlib.Path("dist/modes/rpc/rpc-mode.js")
original = (source / relative).read_text()
if hashlib.sha256(original.encode()).hexdigest() != "631697cd35928fc827f4a423538c43ff227b8cbf63c2a2f11060616f55eba6db":
    parser.error("Pi RPC source differs from the regression-tested baseline")
replacements = [
    ("    await rebindSession();\n    registerSignalHandlers();\n    // Handle a single command", "    // Read UI responses during startup, but gate ordinary commands until all\n    // extension session_start handlers have finished.\n    const initialized = Promise.withResolvers();\n    initialized.promise.catch(() => {});\n    // Handle a single command"),
    ("        const command = parsed;\n        try {\n            const response = await handleCommand(command);", "        const command = parsed;\n        try {\n            await initialized.promise;\n            const response = await handleCommand(command);"),
    ("    // Keep process alive forever\n    return new Promise(() => { });", "    registerSignalHandlers();\n    try {\n        await rebindSession();\n        initialized.resolve();\n    } catch (error) {\n        initialized.reject(error);\n        throw error;\n    }\n    // Keep process alive forever\n    return new Promise(() => { });"),
]
patched = original
for before, after in replacements:
    if patched.count(before) != 1:
        parser.error("patch anchor is not unique")
    patched = patched.replace(before, after)
if hashlib.sha256(patched.encode()).hexdigest() != "bfa1602ef6dce98558358d3df5591d80eb16032334840f289eb5e18ae3013979":
    parser.error("repaired source hash mismatch")
shutil.copytree(source, destination, symlinks=True)
(destination / relative).write_text(patched)
print("Prepared isolated Pi runtime. Use node with its dist/cli.js, NOT dist/bundle/cli.js.")
print(destination / "dist/cli.js")
