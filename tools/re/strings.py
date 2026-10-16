"""Printable strings matching a regex, with first offset and count.
usage: strings.py FILE REGEX MINLEN   e.g. strings.py speed2.exe "REFLECT" 5"""
import re, sys, collections
data = open(sys.argv[1], 'rb').read()
pat = re.compile(rb'[\x20-\x7e]{%d,}' % int(sys.argv[3]))
flt = re.compile(sys.argv[2].encode(), re.I)
c = collections.Counter()
first = {}
for m in pat.finditer(data):
    s = m.group()
    if flt.search(s):
        c[s] += 1
        first.setdefault(s, m.start())
for s, n in sorted(c.items(), key=lambda x: first[x[0]])[:80]:
    print(f"{first[s]:#010x} {n:4} {s.decode()}")
