# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""A container job's steps must not use bash-only shell syntax.

A job with `container:` gets `sh -e {0}` as its default shell, and in the
Debian images this repo builds in, /bin/sh is dash. dash has no `pipefail`
and exits 2 on `set -o pipefail` with "Illegal option -o pipefail".

This is a release-blocking class of bug rather than an ordinary one, because
`release.yml` runs only on a tag push: pull-request CI never executes it, and
the release train builds the .deb by replaying the profile's recipe by hand
rather than by running the workflow. So nothing exercises these steps until
a tag is pushed, and by then the tag exists. That is exactly what happened on
2026-09-19 -- the v0.4.0 tag build failed on both Linux arches, no GitHub
Release was produced, and the tag had to be moved.

A step that genuinely needs bash may have it, by declaring `shell: bash`
(the Debian images do ship bash); this test only rejects bash syntax in a
step that has not asked for bash.
"""

import pathlib
import re

import pytest
import yaml

ROOT = pathlib.Path(__file__).resolve().parents[2]
WORKFLOWS = sorted((ROOT / ".github" / "workflows").glob("*.yml"))

# (pattern, what to say). Only constructs that dash actually rejects or
# silently mis-parses; this is a guard, not a style checker.
BASH_ONLY = [
    (re.compile(r"\bset\s+-[a-z]*o\s+pipefail\b"), "set -o pipefail (dash: Illegal option)"),
    (re.compile(r"\bset\s+-[a-z]*\bo\b"), "set -o (dash has no -o options)"),
    (re.compile(r"\[\["), "[[ ]] test (dash: not found)"),
    (re.compile(r"<<<"), "<<< here-string (dash: unsupported)"),
    (re.compile(r"\$\{[A-Za-z_][A-Za-z0-9_]*:\d"), "${VAR:offset} slicing (dash: bad substitution)"),
]


COMMENT = re.compile(r"^\s*#")


def code_only(run: str) -> str:
    """`run` with whole-line comments dropped.

    A comment that discusses the banned syntax -- including the one above the
    step this guard was written for -- is prose, not something dash will ever
    execute, and failing CI over it would only teach people to stop explaining
    themselves. A `#` inside a quoted string on a code line is not handled;
    this is a guard, not a shell parser.
    """
    return "\n".join(line for line in run.splitlines() if not COMMENT.match(line))


def container_steps():
    """(workflow, job, step name, run block) for every sh-shelled container step."""
    found = []
    for path in WORKFLOWS:
        doc = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        for job_name, job in (doc.get("jobs") or {}).items():
            if not isinstance(job, dict) or not job.get("container"):
                continue
            # A job-level default shell applies to every step that has none.
            job_shell = ((job.get("defaults") or {}).get("run") or {}).get("shell")
            if job_shell:
                continue
            for i, step in enumerate(job.get("steps") or []):
                if not isinstance(step, dict) or "run" not in step:
                    continue
                if step.get("shell"):
                    continue
                name = step.get("name") or f"step {i}"
                found.append((path.name, job_name, name, step["run"]))
    return found


def test_there_are_container_steps_to_check():
    """A guard that silently checks nothing is worse than no guard."""
    steps = container_steps()
    assert steps, "no container-job steps found -- has the workflow layout changed?"


@pytest.mark.parametrize(
    "workflow,job,step,run",
    container_steps(),
    ids=lambda v: v if isinstance(v, str) and len(v) < 40 else "",
)
def test_container_step_avoids_bash_only_syntax(workflow, job, step, run):
    hits = [why for pattern, why in BASH_ONLY if pattern.search(code_only(run))]
    assert not hits, (
        f"{workflow}: job '{job}', step '{step}' runs under dash "
        f"(container job, no `shell:`) but uses: {', '.join(hits)}. "
        f"Use POSIX syntax, or add `shell: bash` to the step."
    )


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__]))
