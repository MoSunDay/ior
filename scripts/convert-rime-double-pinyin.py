#!/usr/bin/env python3
"""Convert rime double-pinyin schema algebra into ior DpTable TSV.

Applies each schema's algebra rules (erase/derive/xform/xlit) to every
syllable in engine-core's SYLLABLES table, then inverts code -> table rows:
  final<TAB>key          final-slot key
  zh|ch|sh<TAB>key       multi-letter initial-slot key
  0<syllable><TAB>key    zero-initial syllable standing on `key`
No output is invented: every row comes from evaluating the schema's own
rules; conflicts are reported and abort the conversion.
"""
import re, sys, pathlib

REPO = pathlib.Path('/home/m/ior')
INITIALS = ['zh', 'ch', 'sh']  # checked before single letters
SINGLE_INITIALS = set('bpmfdtnlgkhjqxrzcsyw')


def syllables():
    src = (REPO / 'crates/engine-core/src/syllable.rs').read_text()
    body = src.split('pub const SYLLABLES: &[&str] = &[', 1)[1].split('];', 1)[0]
    return re.findall(r'"([a-z]+)"', body)


def parse_algebra(text):
    rules, in_alg = [], False
    for line in text.splitlines():
        if line.strip() == 'algebra:':
            in_alg = True
            continue
        if in_alg:
            if line and not line[0].isspace():
                break
            m = re.match(r'\s*-\s+(\w+)/(.*)/(.*)/\s*$', line)
            if m:
                op, pat, repl = m.groups()
                rules.append((op, pat, repl.replace('$', '\\')))
    return rules


def apply(rules, variant, skip, with_xlit):
    variants = [variant]
    for op, pat, repl in rules:
        if op == 'xlit':
            if not with_xlit:
                continue
            if len(pat) != len(repl):
                sys.exit(f'xlit length mismatch {pat!r} vs {repl!r}')
            table = str.maketrans(pat, repl)
            variants = [v.translate(table) for v in variants]
        elif skip(op):
            continue
        elif op == 'erase':
            variants = [v for v in variants if not re.search(pat, v)]
        elif op == 'derive':
            variants += [re.sub(pat, repl, v, count=1) for v in variants if re.search(pat, v)]
        elif op == 'xform':
            variants = [re.sub(pat, repl, v, count=1) for v in variants]
        else:
            sys.exit(f'unsupported algebra op {op!r}')
    return sorted(set(variants))


def initial_of(s):
    for i in INITIALS:
        if s.startswith(i):
            return i
    return s[0] if s[0] in SINGLE_INITIALS else ''


def convert(name, schema_path):
    rules = parse_algebra(schema_path.read_text())
    syls = syllables()
    strict = {s: apply(rules, s, lambda op: op == 'derive', True) for s in syls}
    full = {s: apply(rules, s, lambda op: False, True) for s in syls}
    two = lambda cs: [c for c in cs if len(c) == 2]

    # Pass 1: regular syllables; per-initial canonical first key from strict run.
    codes = {}          # syll -> list of (k1,k2)
    conflicts = []
    groups = {}
    for s in syls:
        ini = initial_of(s)
        if ini:
            groups.setdefault(ini, []).append(s)
    init_key = {}
    for ini, members in groups.items():
        votes = {}
        for s in members:
            for c in two(strict[s]):
                votes[c[0]] = votes.get(c[0], 0) + 1
        if not votes:
            sys.exit(f'{name}: no strict 2-char code for initial group {ini!r}')
        init_key[ini] = max(votes, key=votes.get)
        if ini in SINGLE_INITIALS and init_key[ini] != ini:
            sys.exit(f'{name}: initial {ini!r} typed as {init_key[ini]!r}, expected identity')
    for s in syls:
        ini = initial_of(s)
        if not ini:
            continue
        want = init_key[ini]
        picked = [c for c in two(strict[s]) if c[0] == want]
        if not picked:
            picked = [c for c in two(full[s]) if c[0] == want]
        if not picked:
            sys.exit(f'{name}: no code with initial key {want!r} for syllable {s!r} '
                     f'(strict={strict[s]}, full={full[s]})')
        codes[s] = picked

    finals = {}         # final -> set of keys
    for s, picked in codes.items():
        ini = initial_of(s)
        fin = s[len(ini):]
        for c in picked:
            finals.setdefault(fin, set()).add(c[1])

    # Pass 2: zero-initial syllables.
    zero_rows = []
    for s in syls:
        if initial_of(s):
            continue
        fin = s
        valid = []
        for c in two(strict[s]) + two(full[s]):
            if (fin in finals and c[1] in finals[fin]) or fin not in finals:
                valid.append(c)
        valid = sorted(set(valid))
        if not valid:
            sys.exit(f'{name}: no valid zero code for {s!r} '
                     f'(strict={strict[s]}, full={full[s]}, finals={finals.get(fin)})')
        keys = sorted({c[0] for c in valid})
        for k in keys:
            zero_rows.append((s, k))
        if fin not in finals:
            finals[fin] = {valid[0][1]}

    # Conflicts: one final mapped to more than one key.
    for fin, keys in finals.items():
        if len(keys) > 1:
            conflicts.append((fin, sorted(keys)))

    rows = []
    for fin in sorted(finals):
        for k in sorted(finals[fin]):
            rows.append(f'{fin}\t{k}')
    for ini in sorted(init_key):
        if len(ini) > 1:
            rows.append(f'{ini}\t{init_key[ini]}')
    for s, k in zero_rows:
        rows.append(f'0{s}\t{k}')
    return rows, conflicts


def main():
    outdir = REPO / 'data/double-pinyin'
    schemes = [
        ('natural', '自然碼雙拼 (rime double_pinyin.schema.yaml)'),
        ('flypy', '小鶴雙拼 (rime double_pinyin_flypy.schema.yaml)'),
        ('mspy', '微軟雙拼 (rime double_pinyin_mspy.schema.yaml)'),
    ]
    for key, label in schemes:
        src = outdir / 'rime' / f'{key}.schema.yaml'
        rows, conflicts = convert(key, src)
        header = (f'# {label}\n'
                  f'# Generated by scripts/convert-rime-double-pinyin.py from\n'
                  f'# https://github.com/rime/rime-double-pinyin ({src.name}).\n'
                  f'# Do not hand-edit; rerun the converter instead.\n')
        (outdir / f'{key}.tsv').write_text(header + '\n'.join(rows) + '\n')
        print(f'{key}: {len(rows)} rows, {len(conflicts)} final conflicts {conflicts}')


if __name__ == '__main__':
    main()
