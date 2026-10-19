# Run each scenario in a directory through katzenqt's simulator and check it prints the same.
import sys, pathlib
from katzenqt.groupchat import sim
bad = 0
files = sorted(pathlib.Path(sys.argv[1]).glob("*.txt"))
for p in files:
    text = p.read_text()
    try:
        out = sim.run(text)
    except Exception as e:
        bad += 1; print(f"{p.name}: katzenqt raised {e!r}"); continue
    if out != text:
        bad += 1
        a, b = out.splitlines(), text.splitlines()
        for i, (x, y) in enumerate(zip(a, b)):
            if x != y:
                print(f"{p.name}:{i+1}: go {y!r} katzenqt {x!r}"); break
        else:
            print(f"{p.name}: lengths differ")
print(f"{len(files) - bad} of {len(files)} random scripts print the same in katzenqt")
sys.exit(1 if bad else 0)
