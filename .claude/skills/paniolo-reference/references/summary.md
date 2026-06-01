This file is a merged representation of a subset of the codebase, containing files not matching ignore patterns, combined into a single document by Repomix.

# Summary

## Purpose

This is a reference codebase organized into multiple files for AI consumption.
It is designed to be easily searchable using grep and other text-based tools.

## File Structure

This skill contains the following reference files:

| File | Contents |
|------|----------|
| `project-structure.md` | Directory tree with line counts per file |
| `files.md` | All file contents (search with `## File: <path>`) |
| `tech-stacks.md` | Languages, frameworks, and dependencies per package (search with `## Tech Stack: <path>`) |
| `summary.md` | This file - purpose and format explanation |

## Usage Guidelines

- This file should be treated as read-only. Any changes should be made to the
  original repository files, not this packed version.
- When processing this file, use the file path to distinguish
  between different files in the repository.
- Be aware that this file may contain sensitive information. Handle it with
  the same level of security as you would the original repository.

## Notes

- Some files may have been excluded based on .gitignore rules and Repomix's configuration
- Binary files are not included in this packed representation. Please refer to the Repository Structure section for a complete list of file paths, including binary files
- Files matching these patterns are excluded: captures/**, .git/**, .claude/skills/**
- Files matching patterns in .gitignore are excluded
- Files matching default ignore patterns are excluded
- Files are sorted by Git change count (files with more changes are at the bottom)

## Statistics

94 files | 20,555 lines

| Language | Files | Lines |
|----------|------:|------:|
| Python | 25 | 7,085 |
| Markdown | 23 | 4,349 |
| Rust | 22 | 7,523 |
| No Extension | 10 | 397 |
| TOML | 5 | 261 |
| JavaScript | 2 | 4 |
| LOCK | 1 | 1 |
| CSS | 1 | 209 |
| JSON | 1 | 6 |
| C | 1 | 156 |
| Other | 3 | 564 |

**Largest files:**
- `hdmicap/vendor/nokhwa-bindings-macos/src/lib.rs` (2,463 lines)
- `src/paniolo/_cli.py` (2,093 lines)
- `AGENTS.md` (775 lines)
- `serialcap/src/capture.rs` (683 lines)
- `src/paniolo/_tftp.py` (558 lines)
- `src/paniolo/_netboot.py` (551 lines)
- `serialcap/src/serial_io.rs` (462 lines)
- `netbootd/src/tftp.rs` (448 lines)
- `hdmicap/src/server.rs` (395 lines)
- `hdmicap/src/capture.rs` (359 lines)