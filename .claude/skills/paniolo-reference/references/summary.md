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

64 files | 13,635 lines

| Language | Files | Lines |
|----------|------:|------:|
| Python | 17 | 4,456 |
| Markdown | 13 | 1,976 |
| Rust | 13 | 5,703 |
| No Extension | 9 | 396 |
| TOML | 4 | 203 |
| JavaScript | 2 | 4 |
| CSS | 1 | 209 |
| JSON | 1 | 6 |
| C | 1 | 156 |
| Swift | 1 | 138 |
| Other | 2 | 388 |

**Largest files:**
- `hdmicap/vendor/nokhwa-bindings-macos/src/lib.rs` (2,463 lines)
- `src/paniolo/_cli.py` (1,388 lines)
- `serialcap/src/capture.rs` (683 lines)
- `AGENTS.md` (603 lines)
- `src/paniolo/_tftp.py` (558 lines)
- `serialcap/src/serial_io.rs` (435 lines)
- `src/paniolo/_netboot.py` (415 lines)
- `hdmicap/src/server.rs` (395 lines)
- `hdmicap/src/capture.rs` (359 lines)
- `src/paniolo/_dhcp.py` (334 lines)