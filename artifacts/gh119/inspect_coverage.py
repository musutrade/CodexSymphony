"""Diagnostic only: preserve missing mappings; never a Gate/acceptance result."""
import collections, hashlib, json, pathlib, subprocess, sys
sys.path.insert(0, '/home/gem/.local/share/harness-gate/rust-source/0.1.0-rc.5')
from measure import line_coverage, point_offset, crap
root = pathlib.Path.cwd()
llvm_path = pathlib.Path(sys.argv[1])
native = collections.defaultdict(list)
for unit in json.loads(llvm_path.read_text())['data']:
    for fn in unit['functions']:
        if fn['regions']:
            first = fn['regions'][0]
            native[fn['filenames'][first[5]], tuple(first[:2])].append(fn)
rows = []
for path in pathlib.Path('apps/server/src').rglob('*.rs'):
    source = path.read_bytes()
    items = json.loads(subprocess.check_output(['/home/gem/.local/share/harness-gate/rust-source/0.1.0-rc.5/inventory', str(path)]))
    spans = [(point_offset(source, f['start']), point_offset(source, f['end'])) for f in items]
    for index, fn in enumerate(items):
        records = []
        for anchor in set(map(tuple, fn['anchors'])):
            records += native[str(root/path), anchor]
        row = {'file':str(path), 'sha256':hashlib.sha256(source).hexdigest(), 'name':fn['name'], 'line':fn['start'][0], 'complexity':fn['complexity']}
        if not records:
            row['status'] = 'missing_native_mapping'
            rows.append(row)
            continue
        bounds = spans[index]
        excluded = [span for i, span in enumerate(spans) if i != index and bounds[0] <= span[0] and span[1] <= bounds[1]]
        regions = {}
        invalid = False
        for record in records:
            if fn['asynchronous'] and record['regions'][0][:2] != fn['body']:
                continue
            for sl, sc, el, ec, count, file_id, _, kind in record['regions']:
                if record['filenames'][file_id] != str(root/path) or kind != 0:
                    invalid = True
                    continue
                a, b = point_offset(source, [sl,sc]), point_offset(source, [el,ec])
                if not bounds[0] <= a <= b <= bounds[1]:
                    invalid = True
                    continue
                if a == b or any(x <= a and b <= y for x,y in excluded):
                    continue
                regions[a,b] = max(regions.get((a,b),0), count)
        covered, total = line_coverage(source, [(a,b,n) for (a,b),n in regions.items()], excluded)
        row.update(status='invalid_mapping' if invalid else 'diagnostic_only', covered=covered, total=total, regions_covered=sum(n>0 for n in regions.values()), regions_total=len(regions), crap=crap(fn['complexity'], covered, total))
        rows.append(row)
result = {'acceptance':False, 'coverage_sha256':hashlib.sha256(llvm_path.read_bytes()).hexdigest(), 'note':'Diagnostic of available mappings only. Missing or invalid mappings are not passing measurements. Complete pinned Gate remains mandatory.', 'functions':rows}
pathlib.Path(sys.argv[2]).write_text(json.dumps(result, indent=2)+'\n')
for row in rows:
    if '/environment' in row['file'] and (row['status'] != 'diagnostic_only' or row['covered']*5 < row['total']*4 or row['regions_covered']*5 < row['regions_total']*4):
        print(row)
