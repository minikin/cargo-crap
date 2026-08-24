#!/usr/bin/env bash
# Read a spec's Tasks section as a dependency graph.
#
# Usage: keeler-graph.sh <spec.md>
#
# Prints one line per task, in the order the spec lists them:
#
#     <id> <state> [needs...]
#
# where state is `done` (its box is ticked), `ready` (every need is done)
# or `blocked` (some need is not). Exit 0 with that report, or exit 1
# naming the line and what is wrong with it — a Needs: naming no task, an
# id defined twice, two Needs: in one item, a cycle — and print nothing as
# ready.
#
# The grammar is deliberately small:
#   - the Tasks section runs from `## Tasks` to the next `## ` heading;
#     nothing outside it is a task;
#   - an item runs from a line beginning `- [ ]` or `- [x]` to the next
#     such line, however many physical lines it wraps across;
#   - the item opens with `**Tn — `; Tn is the id;
#   - `Needs: Ta, Tb.` may appear once, anywhere in the item; absent means
#     a root; the checkbox is the only completion signal;
#   - the graph the needs draw is acyclic. A cycle is refused naming its
#     path — no task on it could ever be ready, and a report that called
#     its members blocked would look like a graph waiting on work rather
#     than one that can never finish.
#
# `/keeler:graph`, `just keeler-graph` and `just keeler-spawn` all call
# this and nothing else reads the format — so a human and the tools cannot
# disagree about what the spec's graph is.
#
# All of it is one awk program: macOS ships bash 3.2, which has no
# associative arrays, and awk has had them since 1977.
set -euo pipefail

spec="${1:?usage: keeler-graph.sh <spec.md>}"
[ -f "$spec" ] || { echo "keeler-graph: $spec is not a file" >&2; exit 1; }

awk -v spec="$spec" '
function fail(line, why) {
    printf "keeler-graph: %s line %d: %s\n", spec, line, why > "/dev/stderr"
    # `exit` inside a rule still runs END, which would print the report —
    # and a refusal must report nothing. The flag tells END to stay quiet.
    refused = 1
    exit 1
}

# Close the item being read: parse it, record it, refuse what is wrong.
function close_item(    id, rest, cnt, list, m, k, need) {
    if (!open) return
    open = 0
    if (match(text, /^- \[[ xX]\] \*\*T[0-9]+ /) == 0)
        fail(open_line, "an item that does not open with **Tn — : " substr(text, 1, 60))
    id = substr(text, RSTART + 8, RLENGTH - 9)
    if (id in seen) fail(open_line, id " is defined twice (first at line " seen[id] ")")
    seen[id] = open_line

    cnt = gsub(/Needs:/, "Needs:", text)
    if (cnt > 1) fail(open_line, id " carries two Needs: lines — which one is the graph?")
    # A lowercase slip is not a root, it is a typo; say so rather than
    # read a smaller graph than the one written.
    if (cnt == 0 && text ~ /[Nn]eeds:/)
        fail(open_line, id " has a lowercase needs: — the token is Needs:")
    list = ""
    if (cnt == 1) {
        rest = text
        sub(/.*Needs:[ \t]*/, "", rest)
        sub(/\.([ \t].*)?$/, "", rest)
        gsub(/,/, " ", rest)
        k = split(rest, m, " ")
        delete dup
        for (j = 1; j <= k; j++) {
            if (m[j] !~ /^T[0-9]+$/)
                fail(open_line, id " needs \"" m[j] "\", which is not a task id")
            if (m[j] == id) fail(open_line, id " needs itself")
            if (m[j] in dup) fail(open_line, id " names " m[j] " twice in its Needs:")
            dup[m[j]] = 1
        }
        list = rest
    }
    n++
    ids[n] = id; lines[n] = open_line; ticked[n] = tick; needs[n] = list
    need_of[id] = list
}

# Walk the needs from one task, depth first, and refuse on the first edge
# back into the path being walked — that path, closed, is the cycle, and
# it is named in full so the human can see where to cut it. Colour 1 is
# on the current path, 2 is finished; awk locals are the parameters after
# the real ones, and `m` is local so the recursion does not share it.
function visit(id,    k, j, m, cycle) {
    if (colour[id] == 2) return
    if (colour[id] == 1) {
        cycle = ""
        for (j = at[id]; j <= depth; j++) cycle = cycle path[j] " -> "
        fail(seen[id], "a cycle: " cycle id " — no task on it can ever be ready")
    }
    colour[id] = 1; path[++depth] = id; at[id] = depth
    k = split(need_of[id], m, " ")
    for (j = 1; j <= k; j++) visit(m[j])
    depth--; colour[id] = 2
}

# CRLF endings would hide every heading and every `.` terminator; strip
# the carriage return before anything else looks at the line.
{ sub(/\r$/, "") }

# Fences are tracked over the whole file, before anything else reads a
# line — a spec that quotes the format in its Context (a fenced `## Tasks`
# with an example item under it) would otherwise *open* the section there,
# read the example as the graph, and never reach the real section. Inside
# a fence nothing is a heading, a task or a boundary: it is prose.
/^[ \t]*```/ { close_item(); if (!in_fence) fence_line = NR; in_fence = !in_fence; next }
in_fence            { next }

/^## Tasks[ \t]*$/ { in_tasks = 1; found_section = 1; next }
# A heading at the section level or above ends it — `## Notes`, or a
# `# Appendix` whose checkboxes would otherwise read as more tasks. A
# deeper one does not: `### Phase 1` is structure *within* the section,
# and truncating there would drop tasks in silence, which is the worst
# shape this can take — a graph reporting fewer tasks than exist reports
# every one of them done.
!in_tasks           { next }

in_tasks && /^#{1,2} / { close_item(); in_tasks = 0; next }

/^- \[[ xX]\] /     {
    close_item()
    open = 1; open_line = NR; blank_after = 0
    tick = ($0 ~ /^- \[[xX]\]/) ? 1 : 0
    text = $0
    next
}
# A checkbox line the grammar does not know is a task that would vanish:
# never spawned, never counted at land time. Refuse it, do not drop it —
# but a bullet whose brackets are a markdown link (`](`) is prose, and a
# refusal is total: one link would block the graph, the spawn, the board
# and the land for the whole spec.
/^[-*+] \[/ && !/^[-*+] \[[^]]*\]\(/ {
    fail(NR, "a checkbox line the grammar cannot read: " substr($0, 1, 60))
}
open && /^[ \t]+[^ \t]/ { sub(/^[ \t]+/, " "); text = text $0; next }
open && /^[ \t]*$/      { blank_after = 1; next }
# The lazy continuation markdown allows: an unindented, non-blank, non-heading
# line straight after an item is still the item. An editor reflow puts
# a Needs: there, and dropping it silently would report a root that is
# not one.
open && !blank_after && !/^- / && !/^## / { text = text " " $0; next }
open                    { close_item() }

END {
    if (refused) exit 1
    # A fence nobody closed swallows every line after it, so the section
    # never ends and every task below is dropped — and a graph reporting
    # fewer tasks than exist reports every one of them done, which is a
    # spec `keeler-land` marks Implemented. Refuse instead.
    if (in_fence) fail(fence_line, "a code fence opened here and was never closed — every task after it would be dropped in silence")
    close_item()
    if (!found_section) fail(0, "no ## Tasks section found — nothing to read")
    # Every need must name a task in this spec.
    for (i = 1; i <= n; i++) {
        k = split(needs[i], m, " ")
        for (j = 1; j <= k; j++)
            if (!(m[j] in seen)) fail(lines[i], ids[i] " needs " m[j] ", which no task defines")
    }
    # And the graph they draw must be acyclic — checked only once every
    # need is known to name a task, so the walk never steps off the graph.
    for (i = 1; i <= n; i++) visit(ids[i])
    for (i = 1; i <= n; i++) if (ticked[i]) done_[ids[i]] = 1
    for (i = 1; i <= n; i++) {
        state = "ready"
        if (ids[i] in done_) state = "done"
        else {
            k = split(needs[i], m, " ")
            for (j = 1; j <= k; j++) if (!(m[j] in done_)) { state = "blocked"; break }
        }
        line = ids[i] " " state
        k = split(needs[i], m, " ")
        for (j = 1; j <= k; j++) line = line " " m[j]
        report = report line "\n"
    }
    # Only now, and only if nothing above refused: a malformed section
    # must report nothing as ready, so nothing is printed until it all is.
    if (!refused) printf "%s", report
}
' "$spec"
