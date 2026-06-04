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

131 files | 31,972 lines

| Language | Files | Lines |
|----------|------:|------:|
| Rust | 41 | 15,354 |
| Python | 34 | 9,610 |
| Markdown | 25 | 5,173 |
| No Extension | 14 | 482 |
| TOML | 8 | 399 |
| JavaScript | 2 | 4 |
| LOCK | 1 | 1 |
| CSS | 1 | 209 |
| JSON | 1 | 6 |
| C | 1 | 156 |
| Other | 3 | 578 |

**Largest files:**
- `src/paniolo/_cli.py` (2,600 lines)
- `hdmicap/vendor/nokhwa-bindings-macos/src/lib.rs` (2,463 lines)
- `cli/src/main.rs` (2,341 lines)
- `netbootd/src/tftp.rs` (919 lines)
- `AGENTS.md` (871 lines)
- `serialcap/src/capture.rs` (683 lines)
- `serialcap/src/serial_io.rs` (614 lines)
- `src/paniolo/_netboot.py` (601 lines)
- `cli/src/labfile.rs` (587 lines)
- `src/paniolo/_tftp.py` (563 lines)