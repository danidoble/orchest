"""Bundle PHP's non-glibc shared libraries next to relocatable release binaries."""
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

root = Path(sys.argv[1])
libraries = root / "lib"
queue = [root / "bin/php", root / "sbin/php-fpm"]
seen = set()
glibc = (
    "libc.so.", "libm.so.", "libpthread.so.", "libdl.so.",
    "librt.so.", "libresolv.so.", "ld-linux", "libnss_",
)

while queue:
    binary = queue.pop()
    resolved = binary.resolve()
    if resolved in seen:
        continue
    seen.add(resolved)
    report = subprocess.check_output(["ldd", str(binary)], text=True)
    if "not found" in report:
        raise RuntimeError(f"Missing shared library for {binary}:\n{report}")
    for line in report.splitlines():
        match = re.search(r"=>\s+(/\S+)", line)
        if not match:
            continue
        source = Path(match.group(1))
        if source.name.startswith(glibc):
            continue
        target = libraries / source.name
        if not target.exists():
            shutil.copy2(source, target, follow_symlinks=True)
            os.chmod(target, 0o755)
            subprocess.check_call(["patchelf", "--set-rpath", "$ORIGIN", str(target)])
        queue.append(source)
