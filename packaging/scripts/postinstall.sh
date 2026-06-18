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

# Advisory only — never reaps daemons. paniolo's daemons (serialcap, hdmicap,
# netbootd) are per-user processes, not packaged system services, so a root
# maintainer script can't reliably stop another user's running daemons (their
# state lives under per-user runtime dirs, and killing an in-flight netboot on
# a routine upgrade would be surprising). After an upgrade, any daemon still
# running keeps executing the OLD binary until restarted. The CLI detects this
# (`paniolo daemons` flags such daemons "stale") and heals it on demand.
echo "paniolo: upgrade complete. Any capture daemons already running still use the"
echo "         previous binary — restart them with: paniolo daemons restart --stale"
echo "         (netbootd: restart via 'paniolo netboot start' when convenient)."

exit 0
