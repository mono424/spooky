#!/bin/bash
set -euo pipefail

mkdir -p bench_history

GIT_HASH=$(git rev-parse --short HEAD 2>/dev/null || echo "nogit")
GIT_DIRTY=$(git diff --quiet 2>/dev/null && echo "" || echo "-dirty")
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
FILENAME="bench_history/bench_${TIMESTAMP}_${GIT_HASH}${GIT_DIRTY}.json"
RAW_FILE=$(mktemp)
trap "rm -f $RAW_FILE" EXIT

echo "Running benchmarks..."
cargo bench -q --bench memory_bench 2>&1 | tee "$RAW_FILE"

echo ""
echo "Parsing results into $FILENAME ..."

python3 - "$RAW_FILE" "$TIMESTAMP" "${GIT_HASH}${GIT_DIRTY}" "$FILENAME" << 'PYEOF'
import sys, json, re

raw_file, timestamp, git_hash, out_file = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]

with open(raw_file) as f:
    raw = f.read()

results = []

def parse_time(s):
    m = re.search(r'([\d.]+)\s*(ns|µs|μs|ms|s)\b', s)
    if not m:
        return None
    val = float(m.group(1))
    unit = m.group(2)
    return val * {'ns': 1, 'µs': 1e3, 'μs': 1e3, 'ms': 1e6, 's': 1e9}.get(unit, 1)

def parse_int(s):
    m = re.search(r'(\d+)', s.replace(' ', ''))
    return int(m.group(1)) if m else None

def parse_size(s):
    m = re.search(r'([\d.]+)\s*(B|KB|MB|GB)\b', s)
    if not m:
        return None
    val = float(m.group(1))
    return val * {'B': 1, 'KB': 1024, 'MB': 1024**2, 'GB': 1024**3}.get(m.group(2), 1)

current_bench = None
current_alloc_section = None

for line in raw.splitlines():
    line = re.sub(r'\x1b\[[0-9;]*m', '', line)

    # Match: ├─ bench_name  21.75 µs  │ ...
    bench_match = re.match(r'^.*[├╰]─\s+(\S+)\s+([\d.]+\s*(?:ns|µs|μs|ms|s))\s*│', line)
    if bench_match:
        name = bench_match.group(1)
        parts = [p.strip() for p in line.split('│')]

        unit_match = re.search(r'[\d.]+\s*(ns|µs|μs|ms|s)\b', parts[2] if len(parts) > 2 else parts[0])

        current_bench = {
            'name': name,
            'fastest_ns': parse_time(parts[0]) if len(parts) > 0 else None,
            'slowest_ns': parse_time(parts[1]) if len(parts) > 1 else None,
            'median_ns':  parse_time(parts[2]) if len(parts) > 2 else None,
            'mean_ns':    parse_time(parts[3]) if len(parts) > 3 else None,
            'samples':    parse_int(parts[4])  if len(parts) > 4 else None,
            'iters':      parse_int(parts[5])  if len(parts) > 5 else None,
            'display_unit': unit_match.group(1).replace('μ', 'µ') if unit_match else 'ns',
            'alloc': {},
        }
        results.append(current_bench)
        current_alloc_section = None
        continue

    if current_bench is None:
        continue

    cleaned = re.sub(r'^[\s│├╰─]+', '', line).strip()

    # Alloc section header: "max alloc:", "alloc:", "dealloc:", "grow:"
    section_match = re.match(r'^(max alloc|alloc|dealloc|grow|shrink):\s*$', cleaned.split('│')[0].strip())
    if section_match:
        current_alloc_section = section_match.group(1).replace(' ', '_')
        current_bench['alloc'][current_alloc_section] = {}
        continue

    # Alloc data lines
    if current_alloc_section and '│' in cleaned:
        parts = [p.strip() for p in cleaned.split('│')]
        first = parts[0].strip()
        if not first:
            continue

        section = current_bench['alloc'][current_alloc_section]
        size_val = parse_size(first)
        if size_val is not None:
            median_size = parse_size(parts[2]) if len(parts) > 2 and parse_size(parts[2]) else size_val
            section['bytes'] = median_size
            section['bytes_display'] = (parts[2] if len(parts) > 2 and parse_size(parts[2]) else first).strip()
        else:
            count = parse_int(first)
            if count is not None:
                section['count'] = parse_int(parts[2]) if len(parts) > 2 and parse_int(parts[2]) else count

for r in results:
    if not r['alloc']:
        del r['alloc']

output = {
    'timestamp': timestamp,
    'git_hash': git_hash,
    'benchmarks': results,
}

with open(out_file, 'w') as f:
    json.dump(output, f, indent=2)

print(f"✓ {len(results)} benchmark(s) recorded\n")
for b in results:
    unit = b.get('display_unit', 'ns')
    median = b.get('median_ns', 0)
    d = {'ns': 1, 'µs': 1e3, 'ms': 1e6, 's': 1e9}.get(unit, 1)
    alloc_info = ''
    if 'alloc' in b and 'alloc' in b['alloc']:
        a = b['alloc']['alloc']
        alloc_info = f"  ({a.get('count', '?')} allocs, {a.get('bytes_display', '?')})"
    print(f"  {b['name']:.<40s} {median/d:.2f} {unit} (median){alloc_info}")
PYEOF

echo ""

# Compare with previous run
PREV=$(ls -t bench_history/*.json 2>/dev/null | sed -n '2p')
if [ -n "$PREV" ]; then
    echo "Comparing with previous: $(basename "$PREV")"
    echo ""
    python3 - "$FILENAME" "$PREV" << 'COMPAREEOF'
import json, sys

with open(sys.argv[1]) as f:
    curr = json.load(f)
with open(sys.argv[2]) as f:
    prev = json.load(f)

prev_map = {b["name"]: b for b in prev.get("benchmarks", [])}

for b in curr.get("benchmarks", []):
    name = b["name"]
    if name in prev_map:
        old = prev_map[name].get("median_ns", 0)
        new = b.get("median_ns", 0)
        if old and new:
            change = ((new - old) / old) * 100
            arrow = "🔴" if change > 5 else "🟢" if change < -5 else "⚪"
            print(f"  {arrow} {name:.<40s} {change:+.1f}%")
    else:
        print(f"  🆕 {name}")
COMPAREEOF
else
    echo "(No previous run to compare against)"
fi
